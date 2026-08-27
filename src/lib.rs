// Library exports for testing
pub mod config;
pub mod error;
pub mod pathtools;

// Re-export commonly used types
pub use config::Config;
pub use error::{AppError, AppResult, AuthErrorKind};