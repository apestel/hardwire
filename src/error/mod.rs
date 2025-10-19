use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use std::fmt;

/// Main application error type
#[allow(dead_code)]
#[derive(Debug)]
pub enum AppError {
    /// Database-related errors
    Database(sqlx::Error),

    /// File system errors
    FileSystem(std::io::Error),

    /// File not found
    FileNotFound(String),

    /// Authentication/Authorization errors
    AuthError(AuthErrorKind),

    /// Validation errors
    ValidationError(String),

    /// Configuration errors
    ConfigError(String),

    /// Task/Worker errors
    TaskError(String),

    /// Rate limit exceeded
    RateLimitExceeded,

    /// Share link not found or invalid
    ShareNotFound(String),

    /// File size limit exceeded
    FileSizeLimitExceeded { max_size: u64, actual_size: u64 },

    /// Too many files in share
    TooManyFiles {
        max_files: usize,
        actual_files: usize,
    },

    /// Internal server error with context
    Internal(anyhow::Error),
}

#[allow(dead_code)]
#[derive(Debug)]
pub enum AuthErrorKind {
    InvalidToken,
    ExpiredToken,
    MissingToken,
    Unauthorized,
    InvalidCredentials,
    OAuthError(String),
}

/// Error response structure for JSON API responses
#[derive(Serialize)]
struct ErrorResponse {
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Database(e) => write!(f, "Database error: {}", e),
            AppError::FileSystem(e) => write!(f, "File system error: {}", e),
            AppError::FileNotFound(path) => write!(f, "File not found: {}", path),
            AppError::AuthError(kind) => write!(f, "Authentication error: {}", kind),
            AppError::ValidationError(msg) => write!(f, "Validation error: {}", msg),
            AppError::ConfigError(msg) => write!(f, "Configuration error: {}", msg),
            AppError::TaskError(msg) => write!(f, "Task error: {}", msg),
            AppError::RateLimitExceeded => write!(f, "Rate limit exceeded"),
            AppError::ShareNotFound(id) => write!(f, "Share link not found: {}", id),
            AppError::FileSizeLimitExceeded {
                max_size,
                actual_size,
            } => {
                write!(
                    f,
                    "File size limit exceeded: max {} bytes, got {} bytes",
                    max_size, actual_size
                )
            }
            AppError::TooManyFiles {
                max_files,
                actual_files,
            } => {
                write!(
                    f,
                    "Too many files in share: max {}, got {}",
                    max_files, actual_files
                )
            }
            AppError::Internal(e) => write!(f, "Internal error: {}", e),
        }
    }
}

impl fmt::Display for AuthErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthErrorKind::InvalidToken => write!(f, "Invalid token"),
            AuthErrorKind::ExpiredToken => write!(f, "Token has expired"),
            AuthErrorKind::MissingToken => write!(f, "Missing authentication token"),
            AuthErrorKind::Unauthorized => write!(f, "Unauthorized"),
            AuthErrorKind::InvalidCredentials => write!(f, "Invalid credentials"),
            AuthErrorKind::OAuthError(msg) => write!(f, "OAuth error: {}", msg),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AppError::Database(e) => Some(e),
            AppError::FileSystem(e) => Some(e),
            AppError::Internal(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_message, details, code) = match &self {
            AppError::Database(e) => {
                // Log the actual error but don't expose DB details to clients
                tracing::error!("Database error: {:?}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Database error occurred".to_string(),
                    None,
                    Some("DB_ERROR".to_string()),
                )
            }
            AppError::FileSystem(e) => {
                tracing::error!("File system error: {:?}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "File system error occurred".to_string(),
                    None,
                    Some("FS_ERROR".to_string()),
                )
            }
            AppError::FileNotFound(path) => (
                StatusCode::NOT_FOUND,
                "File not found".to_string(),
                Some(path.clone()),
                Some("FILE_NOT_FOUND".to_string()),
            ),
            AppError::AuthError(kind) => {
                let status = match kind {
                    AuthErrorKind::MissingToken => StatusCode::UNAUTHORIZED,
                    AuthErrorKind::InvalidToken => StatusCode::UNAUTHORIZED,
                    AuthErrorKind::ExpiredToken => StatusCode::UNAUTHORIZED,
                    AuthErrorKind::Unauthorized => StatusCode::FORBIDDEN,
                    AuthErrorKind::InvalidCredentials => StatusCode::UNAUTHORIZED,
                    AuthErrorKind::OAuthError(_) => StatusCode::BAD_REQUEST,
                };
                (
                    status,
                    kind.to_string(),
                    None,
                    Some("AUTH_ERROR".to_string()),
                )
            }
            AppError::ValidationError(msg) => (
                StatusCode::BAD_REQUEST,
                "Validation failed".to_string(),
                Some(msg.clone()),
                Some("VALIDATION_ERROR".to_string()),
            ),
            AppError::ConfigError(msg) => {
                tracing::error!("Configuration error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Configuration error".to_string(),
                    None,
                    Some("CONFIG_ERROR".to_string()),
                )
            }
            AppError::TaskError(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Task processing error".to_string(),
                Some(msg.clone()),
                Some("TASK_ERROR".to_string()),
            ),
            AppError::RateLimitExceeded => (
                StatusCode::TOO_MANY_REQUESTS,
                "Rate limit exceeded".to_string(),
                Some("Please try again later".to_string()),
                Some("RATE_LIMIT".to_string()),
            ),
            AppError::ShareNotFound(id) => (
                StatusCode::NOT_FOUND,
                "Share link not found".to_string(),
                Some(format!("Share ID: {}", id)),
                Some("SHARE_NOT_FOUND".to_string()),
            ),
            AppError::FileSizeLimitExceeded {
                max_size,
                actual_size,
            } => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "File size limit exceeded".to_string(),
                Some(format!(
                    "Maximum allowed: {} MB, provided: {} MB",
                    max_size / (1024 * 1024),
                    actual_size / (1024 * 1024)
                )),
                Some("FILE_TOO_LARGE".to_string()),
            ),
            AppError::TooManyFiles {
                max_files,
                actual_files,
            } => (
                StatusCode::BAD_REQUEST,
                "Too many files in share".to_string(),
                Some(format!(
                    "Maximum: {}, provided: {}",
                    max_files, actual_files
                )),
                Some("TOO_MANY_FILES".to_string()),
            ),
            AppError::Internal(e) => {
                tracing::error!("Internal error: {:?}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "An internal error occurred".to_string(),
                    None,
                    Some("INTERNAL_ERROR".to_string()),
                )
            }
        };

        let body = Json(ErrorResponse {
            error: error_message,
            details,
            code,
        });

        (status, body).into_response()
    }
}

// Conversion implementations for common error types
impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        AppError::Database(err)
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::FileSystem(err)
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Internal(err)
    }
}

// Helper type alias for Results using AppError
#[allow(dead_code)]
pub type AppResult<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = AppError::ValidationError("Invalid email".to_string());
        assert_eq!(err.to_string(), "Validation error: Invalid email");
    }

    #[test]
    fn test_file_size_limit_error() {
        let err = AppError::FileSizeLimitExceeded {
            max_size: 1000,
            actual_size: 2000,
        };
        assert!(err.to_string().contains("1000"));
        assert!(err.to_string().contains("2000"));
    }

    #[test]
    fn test_auth_error_display() {
        let err = AppError::AuthError(AuthErrorKind::ExpiredToken);
        assert!(err.to_string().contains("expired"));
    }
}
