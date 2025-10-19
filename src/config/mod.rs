use anyhow::{Context, Result, anyhow};
use std::env;
use std::path::PathBuf;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub auth: AuthConfig,
    pub limits: LimitsConfig,
    pub observability: ObservabilityConfig,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub data_dir: PathBuf,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub path: PathBuf,
    pub max_connections: u32,
    pub min_connections: u32,
    pub acquire_timeout_secs: u64,
}

#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub jwt_secret: String,
    pub jwt_expiry_hours: u64,
    pub google_client_id: String,
    pub google_client_secret: String,
    pub google_redirect_url: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct LimitsConfig {
    pub max_file_size_bytes: u64,
    pub max_files_per_share: usize,
    pub rate_limit_requests_per_minute: u32,
    pub file_indexer_interval_secs: u64,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ObservabilityConfig {
    pub otlp_endpoint: String,
    pub service_name: String,
    pub enable_console_subscriber: bool,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Config {
            server: ServerConfig::from_env()?,
            database: DatabaseConfig::from_env()?,
            auth: AuthConfig::from_env()?,
            limits: LimitsConfig::from_env()?,
            observability: ObservabilityConfig::from_env()?,
        })
    }

    pub fn validate(&self) -> Result<()> {
        // Validate JWT secret strength
        if self.auth.jwt_secret.len() < 32 {
            return Err(anyhow!(
                "JWT_SECRET must be at least 32 characters, got {}",
                self.auth.jwt_secret.len()
            ));
        }

        // Validate data directory exists or can be created
        if !self.server.data_dir.exists() {
            std::fs::create_dir_all(&self.server.data_dir)
                .context("Failed to create data directory")?;
        }

        // Validate port range
        if self.server.port == 0 {
            return Err(anyhow!("Server port cannot be 0"));
        }

        // Validate limits
        if self.limits.max_file_size_bytes == 0 {
            return Err(anyhow!("max_file_size_bytes must be greater than 0"));
        }

        if self.limits.max_files_per_share == 0 {
            return Err(anyhow!("max_files_per_share must be greater than 0"));
        }

        Ok(())
    }
}

impl ServerConfig {
    fn from_env() -> Result<Self> {
        let host =
            env::var("HARDWIRE_HOST").unwrap_or_else(|_| "http://localhost:8080".to_string());
        let port = env::var("HARDWIRE_PORT")
            .unwrap_or_else(|_| "8080".to_string())
            .parse::<u16>()
            .context("HARDWIRE_PORT must be a valid port number")?;

        let data_dir = env::var("HARDWIRE_DATA_DIR")
            .unwrap_or_else(|_| "./data".to_string())
            .into();

        Ok(ServerConfig {
            host,
            port,
            data_dir,
        })
    }
}

impl DatabaseConfig {
    fn from_env() -> Result<Self> {
        let db_path =
            env::var("HARDWIRE_DB_PATH").unwrap_or_else(|_| "./data/db.sqlite".to_string());

        let max_connections = env::var("HARDWIRE_DB_MAX_CONNECTIONS")
            .unwrap_or_else(|_| "10".to_string())
            .parse()
            .context("HARDWIRE_DB_MAX_CONNECTIONS must be a valid number")?;

        let min_connections = env::var("HARDWIRE_DB_MIN_CONNECTIONS")
            .unwrap_or_else(|_| "2".to_string())
            .parse()
            .context("HARDWIRE_DB_MIN_CONNECTIONS must be a valid number")?;

        let acquire_timeout_secs = env::var("HARDWIRE_DB_ACQUIRE_TIMEOUT")
            .unwrap_or_else(|_| "30".to_string())
            .parse()
            .context("HARDWIRE_DB_ACQUIRE_TIMEOUT must be a valid number")?;

        Ok(DatabaseConfig {
            path: db_path.into(),
            max_connections,
            min_connections,
            acquire_timeout_secs,
        })
    }
}

impl AuthConfig {
    fn from_env() -> Result<Self> {
        let jwt_secret =
            env::var("JWT_SECRET").context("JWT_SECRET environment variable is required")?;

        let jwt_expiry_hours = env::var("JWT_EXPIRY_HOURS")
            .unwrap_or_else(|_| "24".to_string())
            .parse()
            .context("JWT_EXPIRY_HOURS must be a valid number")?;

        let google_client_id = env::var("GOOGLE_CLIENT_ID")
            .context("GOOGLE_CLIENT_ID environment variable is required")?;

        let google_client_secret = env::var("GOOGLE_CLIENT_SECRET")
            .context("GOOGLE_CLIENT_SECRET environment variable is required")?;

        let google_redirect_url = env::var("GOOGLE_REDIRECT_URL")
            .unwrap_or_else(|_| "http://localhost:8080/admin/auth/google/callback".to_string());

        Ok(AuthConfig {
            jwt_secret,
            jwt_expiry_hours,
            google_client_id,
            google_client_secret,
            google_redirect_url,
        })
    }
}

impl LimitsConfig {
    fn from_env() -> Result<Self> {
        let max_file_size_mb = env::var("HARDWIRE_MAX_FILE_SIZE_MB")
            .unwrap_or_else(|_| "5120".to_string()) // Default 5GB
            .parse::<u64>()
            .context("HARDWIRE_MAX_FILE_SIZE_MB must be a valid number")?;

        let max_files_per_share = env::var("HARDWIRE_MAX_FILES_PER_SHARE")
            .unwrap_or_else(|_| "100".to_string())
            .parse()
            .context("HARDWIRE_MAX_FILES_PER_SHARE must be a valid number")?;

        let rate_limit_requests_per_minute = env::var("HARDWIRE_RATE_LIMIT_RPM")
            .unwrap_or_else(|_| "60".to_string())
            .parse()
            .context("HARDWIRE_RATE_LIMIT_RPM must be a valid number")?;

        let file_indexer_interval_secs = env::var("HARDWIRE_FILE_INDEXER_INTERVAL")
            .unwrap_or_else(|_| "300".to_string()) // Default 5 minutes
            .parse()
            .context("HARDWIRE_FILE_INDEXER_INTERVAL must be a valid number")?;

        Ok(LimitsConfig {
            max_file_size_bytes: max_file_size_mb * 1024 * 1024,
            max_files_per_share,
            rate_limit_requests_per_minute,
            file_indexer_interval_secs,
        })
    }
}

impl ObservabilityConfig {
    fn from_env() -> Result<Self> {
        let otlp_endpoint = env::var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")
            .or_else(|_| env::var("OTEL_EXPORTER_OTLP_ENDPOINT"))
            .unwrap_or_else(|_| "http://localhost:4318".to_string());

        let service_name = env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "hardwire".to_string());

        let enable_console_subscriber = env::var("TOKIO_CONSOLE")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(false);

        Ok(ObservabilityConfig {
            otlp_endpoint,
            service_name,
            enable_console_subscriber,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwt_secret_validation() {
        let config = Config {
            server: ServerConfig {
                host: "localhost".to_string(),
                port: 8080,
                data_dir: "/tmp/test".into(),
            },
            database: DatabaseConfig {
                path: "/tmp/test.db".into(),
                max_connections: 10,
                min_connections: 2,
                acquire_timeout_secs: 30,
            },
            auth: AuthConfig {
                jwt_secret: "short".to_string(), // Too short
                jwt_expiry_hours: 24,
                google_client_id: "test".to_string(),
                google_client_secret: "test".to_string(),
                google_redirect_url: "http://localhost".to_string(),
            },
            limits: LimitsConfig {
                max_file_size_bytes: 1000,
                max_files_per_share: 10,
                rate_limit_requests_per_minute: 60,
                file_indexer_interval_secs: 300,
            },
            observability: ObservabilityConfig {
                otlp_endpoint: "http://localhost:4318".to_string(),
                service_name: "test".to_string(),
                enable_console_subscriber: false,
            },
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn test_port_validation() {
        let config = Config {
            server: ServerConfig {
                host: "localhost".to_string(),
                port: 0, // Invalid port
                data_dir: "/tmp/test".into(),
            },
            database: DatabaseConfig {
                path: "/tmp/test.db".into(),
                max_connections: 10,
                min_connections: 2,
                acquire_timeout_secs: 30,
            },
            auth: AuthConfig {
                jwt_secret: "this-is-a-very-long-secure-secret-key-for-testing".to_string(),
                jwt_expiry_hours: 24,
                google_client_id: "test".to_string(),
                google_client_secret: "test".to_string(),
                google_redirect_url: "http://localhost".to_string(),
            },
            limits: LimitsConfig {
                max_file_size_bytes: 1000,
                max_files_per_share: 10,
                rate_limit_requests_per_minute: 60,
                file_indexer_interval_secs: 300,
            },
            observability: ObservabilityConfig {
                otlp_endpoint: "http://localhost:4318".to_string(),
                service_name: "test".to_string(),
                enable_console_subscriber: false,
            },
        };

        assert!(config.validate().is_err());
    }
}
