use axum::{
    Json,
    body::Body,
    extract::Request,
    http::{HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{Html, IntoResponse, Response},
};
use serde_json::json;
use std::fmt;

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Clone)]
struct ErrorMessage(String);

/// Browser navigation gets an error page; API requests keep their JSON response.
pub async fn browser_error_pages(request: Request, next: Next) -> Response {
    let wants_html = request
        .headers()
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|accept| {
            accept.split(',').any(|part| {
                let mut parts = part.split(';');
                parts
                    .next()
                    .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/html"))
                    && !parts.any(|parameter| {
                        parameter.trim().strip_prefix("q=").is_some_and(|quality| {
                            quality.trim().parse::<f32>().unwrap_or(0.0) <= 0.0
                        })
                    })
            })
        });
    let is_head = request.method() == Method::HEAD;
    let mut response = next.run(request).await;
    let status = response.status();
    if wants_html && (status.is_client_error() || status.is_server_error()) {
        let message = response
            .extensions()
            .get::<ErrorMessage>()
            .map(|message| message.0.as_str())
            .unwrap_or_else(|| status.canonical_reason().unwrap_or("Request failed"));
        let page = include_str!("../../assets/error.html")
            .replace("{{STATUS}}", &status.as_u16().to_string())
            .replace("{{MESSAGE}}", &super::escape_html(message));
        let page_response = Html(page).into_response();
        response.headers_mut().remove(header::CONTENT_LENGTH);
        response.headers_mut().remove(header::CONTENT_DISPOSITION);
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            page_response.headers()[header::CONTENT_TYPE].clone(),
        );
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response.headers_mut().insert(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
        *response.body_mut() = if is_head {
            Body::empty()
        } else {
            page_response.into_body()
        };
    }
    response
}

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    Unauthorized,
    RequestRejected {
        status: StatusCode,
        message: String,
    },
    Forbidden,
    NotFound(&'static str),
    InvalidRange(u64),
    Internal {
        context: &'static str,
        detail: String,
    },
}

impl ApiError {
    pub fn internal(context: &'static str, error: impl fmt::Display) -> Self {
        Self::Internal {
            context,
            detail: error.to_string(),
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadRequest(message) => f.write_str(message),
            Self::Unauthorized => f.write_str("Invalid credentials"),
            Self::RequestRejected { message, .. } => f.write_str(message),
            Self::Forbidden => f.write_str("Access denied"),
            Self::NotFound(message) => f.write_str(message),
            Self::InvalidRange(_) => f.write_str("Invalid range"),
            Self::Internal { .. } => f.write_str("Internal server error"),
        }
    }
}

impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::RequestRejected { status, .. } => *status,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::InvalidRange(_) => StatusCode::RANGE_NOT_SATISFIABLE,
            Self::Internal { context, detail } => {
                tracing::error!(context, error = %detail, "Request failed");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let message = self.to_string();
        let mut response = (status, Json(json!({"error": message}))).into_response();
        response.extensions_mut().insert(ErrorMessage(message));
        if let Self::InvalidRange(size) = self {
            if let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}")) {
                response.headers_mut().insert(header::CONTENT_RANGE, value);
            }
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::{Router, http::Request as HttpRequest, routing::get};
    use tower::ServiceExt;

    #[tokio::test]
    async fn browser_pages_escape_messages_and_api_clients_keep_json() {
        let router = Router::new()
            .route(
                "/",
                get(|| async { ApiError::BadRequest("<script>alert('test')</script>".into()) }),
            )
            .layer(axum::middleware::from_fn(browser_error_pages));
        for (accept, html) in [
            ("text/html", true),
            ("application/json", false),
            ("*/*", false),
            ("text/html;q=0, application/json", false),
        ] {
            let response = router
                .clone()
                .oneshot(
                    HttpRequest::builder()
                        .uri("/")
                        .header(header::ACCEPT, accept)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let body = to_bytes(response.into_body(), 16384).await.unwrap();
            if html {
                let page = String::from_utf8(body.to_vec()).unwrap();
                assert!(page.contains("&lt;script&gt;"));
                assert!(!page.contains("<script>"));
                assert!(page.contains("Назад"));
            } else {
                assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(&body).unwrap()["error"],
                    "<script>alert('test')</script>"
                );
            }
        }
    }

    #[tokio::test]
    async fn internal_errors_hide_details_and_ranges_keep_required_header() {
        let response =
            ApiError::internal("Read database", "/private/data: secret detail").into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            json!({"error": "Internal server error"})
        );
        let response = ApiError::InvalidRange(123).into_response();
        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes */123");
    }
}
