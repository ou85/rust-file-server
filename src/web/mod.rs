use crate::{
    app::App,
    blob_store,
    domain::{
        BulkDeleteRequest, ChangePasswordRequest, CreateFolderRequest, CreateUserRequest,
        FileMetadata, FolderMetadata, LoginRequest, MoveFilesRequest, RenameFolderRequest,
        RenameUserRequest, UserAccount, UserInfo, UserRole,
    },
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
    routing::{delete, get, post, put},
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
    folder_id: Option<String>,
}

#[derive(serde::Deserialize)]
struct FolderQuery {
    parent_id: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct DeleteFolderQuery {
    recursive: Option<bool>,
}

#[derive(serde::Serialize)]
struct FolderListResponse {
    current: FolderMetadata,
    folders: Vec<FolderMetadata>,
}

#[derive(serde::Deserialize)]
struct UploadQuery {
    relative_path: Option<String>,
    folder_id: Option<String>,
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
        .route(
            "/change-password",
            get(change_password_page).post(change_password),
        )
        .route("/logout", post(logout))
        .route("/health", get(health))
        .route("/storage", get(storage_stats))
        .route("/me", get(current_user))
        .route("/folders", get(list_folders).post(create_folder))
        .route("/folders/{id}", put(rename_folder).delete(delete_folder))
        .route("/admin/users", get(list_users).post(create_user))
        .route("/admin/users/{id}/name", put(rename_user))
        .route(
            "/admin/users/{id}/request-password-change",
            post(request_password_change),
        )
        .route("/assets/icons/{name}", get(icon))
        .route("/files", get(list_files).delete(delete_files))
        .route("/files/move", post(move_files))
        .route("/files/{id}", get(get_file))
        .route("/files/upload", post(upload_files))
        .route("/uploads/{id}/cancel", post(cancel_upload))
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

async fn list_folders(
    jar: CookieJar,
    State(app): State<Arc<App>>,
    query: Result<Query<FolderQuery>, QueryRejection>,
) -> ApiResult<Json<FolderListResponse>> {
    let user = active_user(&jar, &app)?;
    let Query(query) = query.map_err(|error| ApiError::RequestRejected {
        status: error.status(),
        message: error.body_text(),
    })?;
    let current = match query.parent_id.as_deref() {
        Some(id) => app
            .user_folder(&user, id)
            .map_err(|error| ApiError::internal("Read folder", error))?
            .ok_or(ApiError::NotFound("Folder not found"))?,
        None => app
            .user_root_folder(&user)
            .map_err(|error| ApiError::internal("Open root folder", error))?,
    };
    let folders = app
        .metadata
        .list_child_folders(&user.id, Some(&current.id))
        .map_err(|error| ApiError::internal("List folders", error))?;
    Ok(Json(FolderListResponse { current, folders }))
}

async fn create_folder(
    jar: CookieJar,
    State(app): State<Arc<App>>,
    request: Result<Json<CreateFolderRequest>, JsonRejection>,
) -> ApiResult<(StatusCode, Json<FolderMetadata>)> {
    let user = active_user(&jar, &app)?;
    let Json(request) = request.map_err(|error| ApiError::RequestRejected {
        status: error.status(),
        message: error.body_text(),
    })?;
    let name = validate_folder_name(&request.name)?;
    let folder = app
        .metadata
        .create_folder(&user.id, request.parent_id.as_deref(), &name)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    Ok((StatusCode::CREATED, Json(folder)))
}

async fn rename_folder(
    jar: CookieJar,
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    request: Result<Json<RenameFolderRequest>, JsonRejection>,
) -> ApiResult<Json<FolderMetadata>> {
    let user = active_user(&jar, &app)?;
    let Json(request) = request.map_err(|error| ApiError::RequestRejected {
        status: error.status(),
        message: error.body_text(),
    })?;
    let name = validate_folder_name(&request.name)?;
    let folder = app
        .metadata
        .rename_folder(&user.id, &id, &name)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?
        .ok_or(ApiError::NotFound("Folder not found"))?;
    Ok(Json(folder))
}

async fn delete_folder(
    jar: CookieJar,
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    query: Result<Query<DeleteFolderQuery>, QueryRejection>,
) -> ApiResult<Json<serde_json::Value>> {
    let user = active_user(&jar, &app)?;
    let Query(query) = query.map_err(|error| ApiError::RequestRejected {
        status: error.status(),
        message: error.body_text(),
    })?;
    let deleted = app
        .delete_folder_for_user(&user, &id, query.recursive.unwrap_or(false))
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    if !deleted {
        return Err(ApiError::NotFound("Folder not found"));
    }
    Ok(Json(json!({"deleted": true, "id": id})))
}

async fn list_users(jar: CookieJar, State(app): State<Arc<App>>) -> ApiResult<Json<Vec<UserInfo>>> {
    require_admin(&jar, &app)?;
    let users = app
        .metadata
        .list_users()
        .map_err(|error| ApiError::internal("List users", error))?;
    Ok(Json(users.iter().map(UserInfo::from).collect()))
}

async fn create_user(
    jar: CookieJar,
    State(app): State<Arc<App>>,
    request: Result<Json<CreateUserRequest>, JsonRejection>,
) -> ApiResult<(StatusCode, Json<UserInfo>)> {
    require_admin(&jar, &app)?;
    let Json(request) = request.map_err(|error| ApiError::RequestRejected {
        status: error.status(),
        message: error.body_text(),
    })?;
    crate::auth::validate_username(&request.username)
        .map_err(|message| ApiError::BadRequest(message.into()))?;
    let user = UserAccount {
        id: uuid::Uuid::new_v4().to_string(),
        username: request.username,
        password_hash: crate::auth::hash_password(&request.password)
            .map_err(|error| ApiError::BadRequest(error.to_string()))?,
        auth_version: 1,
        role: UserRole::User,
        password_change_required: true,
        encrypted_data_key: app
            .crypto
            .create_wrapped_user_key()
            .map_err(|error| ApiError::internal("Create user encryption key", error))?,
    };
    app.metadata
        .create_user(&user)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    tracing::info!(username = %user.username, "User account created");
    Ok((StatusCode::CREATED, Json(UserInfo::from(&user))))
}

async fn rename_user(
    jar: CookieJar,
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    request: Result<Json<RenameUserRequest>, JsonRejection>,
) -> ApiResult<Json<UserInfo>> {
    require_admin(&jar, &app)?;
    let Json(request) = request.map_err(|error| ApiError::RequestRejected {
        status: error.status(),
        message: error.body_text(),
    })?;
    crate::auth::validate_username(&request.username)
        .map_err(|message| ApiError::BadRequest(message.into()))?;
    let user = app
        .metadata
        .get_user(&id)
        .map_err(|error| ApiError::internal("Read user account", error))?
        .ok_or(ApiError::NotFound("User not found"))?;
    if user.role == UserRole::Admin {
        return Err(ApiError::Forbidden);
    }
    let user = app
        .metadata
        .rename_user(&id, &request.username)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?
        .ok_or(ApiError::NotFound("User not found"))?;
    tracing::info!(user_id = %id, username = %user.username, "User account renamed");
    Ok(Json(UserInfo::from(&user)))
}

async fn request_password_change(
    jar: CookieJar,
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> ApiResult<Json<UserInfo>> {
    require_admin(&jar, &app)?;
    let user = app
        .metadata
        .get_user(&id)
        .map_err(|error| ApiError::internal("Read user account", error))?
        .ok_or(ApiError::NotFound("User not found"))?;
    if user.role == UserRole::Admin {
        return Err(ApiError::Forbidden);
    }
    let user = app
        .metadata
        .require_password_change(&id)
        .map_err(|error| ApiError::internal("Request password change", error))?
        .ok_or(ApiError::NotFound("User not found"))?;
    tracing::info!(user_id = %id, "Password change requested");
    Ok(Json(UserInfo::from(&user)))
}

async fn icon(
    jar: CookieJar,
    State(app): State<Arc<App>>,
    Path(name): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let _user = active_user(&jar, &app)?;
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
        Some(account) if account.role == UserRole::User && account.password_change_required => {
            Redirect::to("/change-password").into_response()
        }
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

async fn change_password_page(jar: CookieJar, State(app): State<Arc<App>>) -> impl IntoResponse {
    match session_user(&jar, &app) {
        Ok(account) if account.role == UserRole::User && account.password_change_required => {
            Html(include_str!("../../assets/change_password.html")).into_response()
        }
        Ok(_) => Redirect::to("/").into_response(),
        Err(_) => Redirect::to("/login").into_response(),
    }
}

async fn change_password(
    jar: CookieJar,
    State(app): State<Arc<App>>,
    request: Result<Json<ChangePasswordRequest>, JsonRejection>,
) -> ApiResult<Response> {
    let account = session_user(&jar, &app)?;
    if account.role != UserRole::User || !account.password_change_required {
        return Err(ApiError::Forbidden);
    }
    let Json(request) = request.map_err(|error| ApiError::RequestRejected {
        status: error.status(),
        message: error.body_text(),
    })?;
    if request.password != request.confirmation {
        return Err(ApiError::BadRequest("Passwords do not match".into()));
    }
    let hash = crate::auth::hash_password(&request.password)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    let account = app
        .metadata
        .change_password(&account.id, &hash)
        .map_err(|error| ApiError::internal("Change password", error))?
        .ok_or(ApiError::Forbidden)?;
    let token = app
        .sessions
        .create(&account)
        .map_err(|error| ApiError::internal("Create replacement session", error))?;
    if let Some(old) = jar.get("rfs_session") {
        app.sessions.remove(old.value());
    }
    tracing::info!(user_id = %account.id, "Password changed");
    Ok((
        StatusCode::OK,
        [(
            header::SET_COOKIE,
            format!("rfs_session={token}; Path=/; HttpOnly; SameSite=Strict"),
        )],
    )
        .into_response())
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
    let user = active_user(&jar, &app)?;
    let Query(query) = query.map_err(|error| ApiError::RequestRejected {
        status: error.status(),
        message: error.body_text(),
    })?;

    let folder = match query.folder_id.as_deref() {
        Some(id) => app
            .user_folder(&user, id)
            .map_err(|error| ApiError::internal("Read folder", error))?
            .ok_or(ApiError::NotFound("Folder not found"))?,
        None => app
            .user_root_folder(&user)
            .map_err(|error| ApiError::internal("Open root folder", error))?,
    };
    match app.list_files_for_user_in_folder(&user, &folder.id) {
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
    let user = active_user(&jar, &app)?;
    Ok(Json(app.get_file_for_user(&user, &id).map_err(
        |error| ApiError::internal("Read file metadata", error),
    )?))
}

async fn delete_file(
    jar: CookieJar,
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
) -> ApiResult<Json<serde_json::Value>> {
    let user = active_user(&jar, &app)?;
    app.delete_file_for_user(&user, &id)
        .map_err(|error| ApiError::internal("Delete file", error))?;
    tracing::info!(file_id = %id, "File deleted");
    Ok(Json(json!({"deleted": true, "id": id})))
}

async fn delete_files(
    jar: CookieJar,
    State(app): State<Arc<App>>,
    request: Result<Json<BulkDeleteRequest>, JsonRejection>,
) -> ApiResult<Json<serde_json::Value>> {
    let user = active_user(&jar, &app)?;
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
        app.delete_file_for_user(&user, &id)
            .map_err(|error| ApiError::internal("Delete selected file", error))?;
        deleted.push(id);
    }
    tracing::info!(count = deleted.len(), "Selected files deleted");
    Ok(Json(json!({ "deleted": deleted })))
}

async fn move_files(
    jar: CookieJar,
    State(app): State<Arc<App>>,
    request: Result<Json<MoveFilesRequest>, JsonRejection>,
) -> ApiResult<Json<Vec<FileMetadata>>> {
    let user = active_user(&jar, &app)?;
    let Json(request) = request.map_err(|error| ApiError::RequestRejected {
        status: error.status(),
        message: error.body_text(),
    })?;
    if request.ids.is_empty() || request.ids.len() > 1_000 {
        return Err(ApiError::BadRequest(
            "Provide between 1 and 1000 file IDs".into(),
        ));
    }
    let files = app
        .metadata
        .move_files_for_owner(&user.id, &request.ids, &request.folder_id)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    Ok(Json(files))
}

fn lookup_file(app: &App, user: &UserAccount, id: &str) -> ApiResult<FileMetadata> {
    app.get_file_for_user(user, id)
        .map_err(|error| ApiError::internal("Read file metadata", error))?
        .ok_or(ApiError::NotFound("File not found"))
}

async fn download_file(
    jar: CookieJar,
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
) -> ApiResult<impl IntoResponse> {
    let user = active_user(&jar, &app)?;

    let metadata = lookup_file(&app, &user, &id)?;
    let chunks = app
        .export_chunked_for_user(&user, &id)
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
    let user = active_user(&jar, &app)?;
    let metadata = lookup_file(&app, &user, &id)?;

    Ok(Redirect::to(&format!("/files/{}/stream", metadata.id)).into_response())
}

async fn video_player(
    jar: CookieJar,
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
) -> ApiResult<impl IntoResponse> {
    let user = active_user(&jar, &app)?;
    let metadata = lookup_file(&app, &user, &id)?;
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
    headers: HeaderMap,
    query: Result<Query<UploadQuery>, QueryRejection>,
    multipart: Result<Multipart, MultipartRejection>,
) -> ApiResult<Json<Vec<FileMetadata>>> {
    let user = active_user(&jar, &app)?;
    let Query(query) = query.map_err(|error| ApiError::RequestRejected {
        status: error.status(),
        message: error.body_text(),
    })?;
    let relative_path = query
        .relative_path
        .map(|path| validate_relative_upload_path(&path))
        .transpose()?;
    let start_folder = match query.folder_id.as_deref() {
        Some(id) => app
            .user_folder(&user, id)
            .map_err(|error| ApiError::internal("Read upload folder", error))?
            .ok_or(ApiError::NotFound("Folder not found"))?,
        None => app
            .user_root_folder(&user)
            .map_err(|error| ApiError::internal("Open upload folder", error))?,
    };
    let upload_id = headers
        .get("x-upload-id")
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            if valid_upload_id(value) {
                Ok(value.to_owned())
            } else {
                Err(ApiError::BadRequest("Invalid upload identifier".into()))
            }
        })
        .transpose()?
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if !app
        .register_upload(&upload_id, &user.id)
        .map_err(|error| ApiError::internal("Register upload", error))?
    {
        return Err(ApiError::BadRequest(
            "Upload identifier is already active".into(),
        ));
    }
    let _upload_guard = UploadCleanup {
        app: app.clone(),
        upload_id: upload_id.clone(),
        blob_ids: Vec::new(),
        keep_files: false,
    };
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
        if !uploaded.is_empty() {
            return Err(ApiError::BadRequest("Upload one file per request".into()));
        }
        use crate::crypto::CHUNK_SIZE;
        use std::io::Write;

        let relative_path = relative_path
            .clone()
            .unwrap_or_else(|| field.file_name().unwrap_or("unknown").to_string());
        let (filename, folder_id) =
            resolve_upload_destination(&app, &user, &start_folder.id, Some(&relative_path))?;
        if app
            .metadata
            .file_name_exists(&user.id, &folder_id, &filename)
            .map_err(|error| ApiError::internal("Check upload destination", error))?
        {
            return Err(ApiError::Conflict("A file with this name already exists"));
        }
        let id = crate::domain::id::id_16();
        // Keep track immediately so cancellation or a disconnected client removes partial data.
        // SAFETY: this guard owns the upload lifecycle until the metadata transaction commits.
        let mut cleanup = UploadCleanup {
            app: app.clone(),
            upload_id: String::new(),
            blob_ids: vec![id.clone()],
            keep_files: false,
        };

        let mut file = app
            .storage
            .create_file_writer(&id)
            .map_err(|error| ApiError::internal("Create upload file", error))?;
        let crypto = app
            .user_crypto(&user)
            .map_err(|error| ApiError::internal("Unlock user encryption key", error))?;
        let mut encryptor = crypto
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
            if app
                .upload_cancelled(&upload_id, &user.id)
                .map_err(|error| ApiError::internal("Check upload cancellation", error))?
            {
                return Err(ApiError::BadRequest("Upload cancelled".into()));
            }
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
            owner_id: Some(user.id.clone()),
            folder_id: Some(folder_id.clone()),
        };

        // Hold the upload state lock through publication so cancellation cannot race commit.
        let committed = app
            .commit_upload(&upload_id, &user.id, || {
                app.storage.finalize_file(&id)?;
                if let Err(error) = app.metadata.save_file(&metadata) {
                    let _ = app.storage.delete_file(&id);
                    return Err(error);
                }
                Ok(())
            })
            .map_err(|error| ApiError::internal("Publish upload", error))?;
        if !committed {
            return Err(ApiError::BadRequest("Upload cancelled".into()));
        }
        cleanup.keep_files = true;
        tracing::info!(file_id = %id, bytes = total_bytes, "Upload completed");

        uploaded.push(metadata);
    }

    if uploaded.is_empty() {
        return Err(ApiError::BadRequest("No files uploaded".into()));
    }

    Ok(Json(uploaded))
}

fn validate_relative_upload_path(path: &str) -> Result<String, ApiError> {
    if path.is_empty() || path.len() > 1024 || path.starts_with('/') || path.contains('\\') {
        return Err(ApiError::BadRequest("Invalid relative upload path".into()));
    }
    let components: Vec<&str> = path.split('/').collect();
    if components.iter().any(|component| {
        component.is_empty()
            || *component == "."
            || *component == ".."
            || component.chars().any(char::is_control)
    }) {
        return Err(ApiError::BadRequest("Invalid relative upload path".into()));
    }
    Ok(path.to_owned())
}

fn validate_folder_name(name: &str) -> Result<String, ApiError> {
    if name.is_empty()
        || name.len() > 255
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.chars().any(char::is_control)
    {
        return Err(ApiError::BadRequest("Invalid folder name".into()));
    }
    Ok(name.to_owned())
}

fn resolve_upload_destination(
    app: &App,
    user: &UserAccount,
    start_folder_id: &str,
    relative_path: Option<&str>,
) -> ApiResult<(String, String)> {
    let path = relative_path.ok_or(ApiError::BadRequest("Missing upload filename".into()))?;
    let path = validate_relative_upload_path(path)?;
    let mut parts = path.split('/').peekable();
    let mut folder_id = start_folder_id.to_owned();
    let mut filename = None;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            filename = Some(part.to_owned());
            break;
        }
        let existing = app
            .metadata
            .list_child_folders(&user.id, Some(&folder_id))
            .map_err(|error| ApiError::internal("List upload folders", error))?
            .into_iter()
            .find(|folder| folder.name == part);
        folder_id = match existing {
            Some(folder) => folder.id,
            None => {
                app.metadata
                    .create_folder(&user.id, Some(&folder_id), part)
                    .map_err(|error| ApiError::BadRequest(error.to_string()))?
                    .id
            }
        };
    }
    Ok((
        filename.ok_or(ApiError::BadRequest("Missing upload filename".into()))?,
        folder_id,
    ))
}

async fn cancel_upload(
    jar: CookieJar,
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let user = active_user(&jar, &app)?;
    if !valid_upload_id(&id) {
        return Err(ApiError::BadRequest("Invalid upload identifier".into()));
    }
    let cancelled = app
        .cancel_upload(&id, &user.id)
        .map_err(|error| ApiError::internal("Cancel upload", error))?
        .unwrap_or(false);
    Ok(Json(json!({ "cancelled": cancelled })))
}

fn valid_upload_id(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .chars()
            .all(|character| character.is_ascii_hexdigit() || character == '-')
}

struct UploadCleanup {
    app: Arc<App>,
    upload_id: String,
    blob_ids: Vec<String>,
    keep_files: bool,
}

impl Drop for UploadCleanup {
    fn drop(&mut self) {
        if !self.keep_files {
            for id in &self.blob_ids {
                if let Err(error) = self.app.storage.delete_tmp_file(id) {
                    tracing::warn!(file_id = %id, error = %error, "Could not remove cancelled upload temporary file");
                }
                if let Err(error) = self.app.storage.delete_file(id) {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        tracing::warn!(file_id = %id, error = %error, "Could not remove cancelled upload blob");
                    }
                }
            }
        }
        if !self.upload_id.is_empty() {
            self.app.remove_upload(&self.upload_id);
        }
    }
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

fn active_user(jar: &CookieJar, app: &App) -> ApiResult<UserAccount> {
    let account = session_user(jar, app)?;
    (account.role == UserRole::User && !account.password_change_required)
        .then_some(account)
        .ok_or(ApiError::Forbidden)
}

fn require_user(jar: &CookieJar, app: &App) -> ApiResult<()> {
    active_user(jar, app).map(|_| ())
}

fn require_admin(jar: &CookieJar, app: &App) -> ApiResult<()> {
    (session_user(jar, app)?.role == UserRole::Admin)
        .then_some(())
        .ok_or(ApiError::Forbidden)
}

async fn stream_file(
    jar: CookieJar,
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let user = active_user(&jar, &app)?;
    let metadata = lookup_file(&app, &user, &id)?;

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
                .export_range_for_user(&user, &id, start, end)
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
        .export_chunked_for_user(&user, &id)
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
                encrypted_data_key: app.crypto.create_wrapped_user_key().unwrap(),
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

    fn admin_cookie(app: &App) -> String {
        format!(
            "rfs_session={}",
            app.sessions
                .create(&app.metadata.get_user_by_username("admin").unwrap().unwrap())
                .unwrap()
        )
    }

    fn account_cookie(app: &App, username: &str) -> String {
        format!(
            "rfs_session={}",
            app.sessions
                .create(
                    &app.metadata
                        .get_user_by_username(username)
                        .unwrap()
                        .unwrap()
                )
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
                owner_id: Some("test-user".into()),
                folder_id: Some("root-test-user".into()),
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
                    owner_id: Some("test-user".into()),
                    folder_id: Some("root-test-user".into()),
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
                owner_id: Some("test-user".into()),
                folder_id: Some("root-test-user".into()),
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
                    .uri("/files/upload?relative_path=project%2Fhello.txt")
                    .header("cookie", user_cookie(&app))
                    .header(
                        "content-type",
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let uploaded: Vec<FileMetadata> =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(uploaded.len(), 1);
        assert_eq!(uploaded[0].filename, "hello.txt");
        let root = app.metadata.root_folder("test-user").unwrap().unwrap();
        let folders = app
            .metadata
            .list_child_folders("test-user", Some(&root.id))
            .unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].name, "project");
        assert_eq!(
            uploaded[0].folder_id.as_deref(),
            Some(folders[0].id.as_str())
        );

        let duplicate = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/files/upload?relative_path=project%2Fhello.txt")
                    .header("cookie", user_cookie(&app))
                    .header(
                        "content-type",
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(duplicate.status(), StatusCode::CONFLICT);
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

    #[tokio::test]
    async fn folder_lifecycle_is_scoped_and_non_empty_folders_are_protected() {
        let (app, dir) = test_app();
        let create_request = |name: &str, parent_id: Option<&str>| {
            let body = serde_json::json!({ "name": name, "parent_id": parent_id });
            Request::builder()
                .method("POST")
                .uri("/folders")
                .header("cookie", user_cookie(&app))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap()
        };
        let response = create_router(app.clone())
            .oneshot(create_request("Documents", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let documents: FolderMetadata =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        let response = create_router(app.clone())
            .oneshot(create_request("Nested", Some(&documents.id)))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let nested: FolderMetadata =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();

        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/folders/{}", documents.id))
                    .header("cookie", user_cookie(&app))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/folders/{}", nested.id))
                    .header("cookie", user_cookie(&app))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let other = UserAccount {
            id: "other-user".into(),
            username: "other".into(),
            password_hash: hash_password("test-password").unwrap(),
            auth_version: 1,
            role: UserRole::User,
            password_change_required: false,
            encrypted_data_key: app.crypto.create_wrapped_user_key().unwrap(),
        };
        app.metadata.create_user(&other).unwrap();
        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .uri(format!("/folders?parent_id={}", documents.id))
                    .header("cookie", account_cookie(&app, "other"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let file = FileMetadata {
            id: "folder-file".into(),
            filename: "inside.txt".into(),
            size: 10,
            created_at: 1,
            owner_id: Some("test-user".into()),
            folder_id: Some(documents.id.clone()),
        };
        app.metadata.save_file(&file).unwrap();
        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/folders/{}", documents.id))
                    .header("cookie", user_cookie(&app))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/folders/{}?recursive=true", documents.id))
                    .header("cookie", user_cookie(&app))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(app.metadata.get_folder(&documents.id).unwrap().is_none());
        assert!(app.metadata.get_file("folder-file").unwrap().is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn selected_files_can_be_moved_to_another_folder() {
        let (app, dir) = test_app();
        let root = app.metadata.root_folder("test-user").unwrap().unwrap();
        let destination = app
            .metadata
            .create_folder("test-user", Some(&root.id), "Archive")
            .unwrap();
        for id in ["move-a", "move-b"] {
            app.metadata
                .save_file(&FileMetadata {
                    id: id.into(),
                    filename: format!("{id}.txt"),
                    size: 1,
                    created_at: 1,
                    owner_id: Some("test-user".into()),
                    folder_id: Some(root.id.clone()),
                })
                .unwrap();
        }
        let body = serde_json::json!({
            "ids": ["move-a", "move-b"],
            "folder_id": destination.id
        });
        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/files/move")
                    .header("cookie", user_cookie(&app))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            app.metadata
                .get_file("move-a")
                .unwrap()
                .unwrap()
                .folder_id
                .as_deref(),
            Some(destination.id.as_str())
        );
        let body = serde_json::json!({
            "ids": ["move-a", "move-b"],
            "folder_id": root.id
        });
        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/files/move")
                    .header("cookie", user_cookie(&app))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            app.metadata
                .get_file("move-a")
                .unwrap()
                .unwrap()
                .folder_id
                .as_deref(),
            Some(root.id.as_str())
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn administrator_can_create_rename_and_require_password_change() {
        let (app, dir) = test_app();
        let router = create_router(app.clone());
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/users")
                    .header("cookie", admin_cookie(&app))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"username":"alice","password":"temporary-password"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let created: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(created["username"], "alice");
        assert_eq!(created["password_change_required"], true);
        let id = created["id"].as_str().unwrap();

        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/admin/users/{id}/name"))
                    .header("cookie", admin_cookie(&app))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"username":"alice-new"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            app.metadata
                .get_user_by_username("alice-new")
                .unwrap()
                .is_some()
        );
        assert!(
            app.metadata
                .get_user_by_username("alice")
                .unwrap()
                .is_none()
        );

        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/admin/users/{id}/request-password-change"))
                    .header("cookie", admin_cookie(&app))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            app.metadata
                .get_user(id)
                .unwrap()
                .unwrap()
                .password_change_required
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn password_change_blocks_file_access_until_completed() {
        let (app, dir) = test_app();
        let account = UserAccount {
            id: "must-change".into(),
            username: "bob".into(),
            password_hash: hash_password("temporary-password").unwrap(),
            auth_version: 1,
            role: UserRole::User,
            password_change_required: true,
            encrypted_data_key: app.crypto.create_wrapped_user_key().unwrap(),
        };
        app.metadata.create_user(&account).unwrap();
        let old_cookie = format!("rfs_session={}", app.sessions.create(&account).unwrap());

        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("cookie", &old_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers()[header::LOCATION], "/change-password");

        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .uri("/files")
                    .header("cookie", &old_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/change-password")
                    .header("cookie", &old_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"password":"new-long-password","confirmation":"new-long-password"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let new_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        assert!(crate::auth::verify_password(
            "new-long-password",
            &app.metadata.get_user("must-change").unwrap().unwrap()
        ));

        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .uri("/files")
                    .header("cookie", new_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn users_cannot_list_or_download_each_others_encrypted_files() {
        let (app, dir) = test_app();
        app.metadata
            .create_user(&UserAccount {
                id: "second-user".into(),
                username: "second".into(),
                password_hash: hash_password("second-user-password").unwrap(),
                auth_version: 1,
                role: UserRole::User,
                password_change_required: false,
                encrypted_data_key: app.crypto.create_wrapped_user_key().unwrap(),
            })
            .unwrap();
        let boundary = "owner-boundary";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"private.txt\"\r\n\r\nprivate content\r\n--{boundary}--\r\n"
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
        let id = &uploaded[0].id;
        assert_eq!(uploaded[0].owner_id.as_deref(), Some("test-user"));

        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .uri("/files")
                    .header("cookie", account_cookie(&app, "second"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(list["total"], 0);

        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .uri(format!("/files/{id}/download"))
                    .header("cookie", account_cookie(&app, "second"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        fs::remove_dir_all(dir).unwrap();
    }
}
