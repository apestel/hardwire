// Library exports for testing
pub mod config;
pub mod error;

// Re-export commonly used types
pub use config::Config;
pub use error::{AppError, AppResult, AuthErrorKind};
