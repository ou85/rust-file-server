use axum::{
    Json,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::json;
use std::fmt;

pub type ApiResult<T> = Result<T, ApiError>;

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
        let mut response = (status, Json(json!({"error": self.to_string()}))).into_response();
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
