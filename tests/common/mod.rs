use hardwire::config::Config;
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};

use tempfile::TempDir;

/// Test context that provides isolated database and filesystem for tests
pub struct TestContext {
    pub db_pool: SqlitePool,
    pub temp_dir: TempDir,
    pub config: Config,
}

impl TestContext {
    /// Create a new test context with in-memory database and temporary directory
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let temp_dir = TempDir::new()?;

        // Create temporary file-based SQLite database for tests to support sqlx migrations
        let db_path = temp_dir.path().join("test.db");
        let opts = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true);

        let db_pool = SqlitePool::connect_with(opts).await?;

        // Run migrations using native sqlx system
        sqlx::migrate!().run(&db_pool).await?;

        // Create test configuration
        let config = create_test_config(temp_dir.path())?;

        Ok(TestContext {
            db_pool,
            temp_dir,
            config,
        })
    }
}

/// Create a test configuration with safe defaults
pub fn create_test_config(
    temp_dir: &std::path::Path,
) -> Result<Config, Box<dyn std::error::Error>> {
    use hardwire::config::*;

    Ok(Config {
        server: ServerConfig {
            host: "http://localhost:8080".to_string(),
            port: 8080,
            data_dir: temp_dir.to_path_buf(),
        },
        database: DatabaseConfig {
            path: temp_dir.join("test.db"),
            max_connections: 5,
            min_connections: 1,
            acquire_timeout_secs: 30,
        },
        auth: AuthConfig {
            jwt_secret: "test-secret-key-with-at-least-32-characters-for-testing".to_string(),
            jwt_expiry_hours: 24,
            google_client_id: "test-client-id".to_string(),
            google_client_secret: "test-client-secret".to_string(),
            google_redirect_url: "http://localhost:8080/admin/auth/google/callback".to_string(),
        },
        limits: LimitsConfig {
            max_file_size_bytes: 1024 * 1024 * 100, // 100MB for tests
            max_files_per_share: 50,
            rate_limit_requests_per_minute: 1000, // High limit for tests
            file_indexer_interval_secs: 60,
        },
        observability: ObservabilityConfig {
            otlp_endpoint: "http://localhost:4318".to_string(),
            service_name: "hardwire-test".to_string(),
            enable_console_subscriber: false,
        },
    })
}

/// Helper to create a test admin user
pub async fn create_test_admin_user(
    pool: &SqlitePool,
    email: &str,
    google_id: &str,
) -> Result<i64, sqlx::Error> {
    let now = chrono::Utc::now().timestamp();
    let result = sqlx::query!(
        "INSERT INTO admin_users (email, google_id, created_at) VALUES (?, ?, ?) RETURNING id",
        email,
        google_id,
        now
    )
    .fetch_one(pool)
    .await?;

    Ok(result.id)
}

/// Helper to create a test file in the database
pub async fn create_test_file(pool: &SqlitePool, path: &str) -> Result<i64, sqlx::Error> {
    let result = sqlx::query!("INSERT INTO files (path) VALUES (?) RETURNING id", path)
        .fetch_one(pool)
        .await?;

    Ok(result.id)
}

/// Helper to create a test share link
pub async fn create_test_share(
    pool: &SqlitePool,
    share_id: &str,
    file_ids: &[i64],
) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().timestamp();
    let expiration = now + (7 * 24 * 60 * 60); // 7 days from now

    // Create share link
    sqlx::query!(
        "INSERT INTO share_links (id, expiration, created_at) VALUES (?, ?, ?)",
        share_id,
        expiration,
        now
    )
    .execute(pool)
    .await?;

    // Associate files with share
    for file_id in file_ids {
        sqlx::query!(
            "INSERT INTO share_link_files (share_link_id, file_id) VALUES (?, ?)",
            share_id,
            file_id
        )
        .execute(pool)
        .await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_context_creation() {
        let ctx = TestContext::new()
            .await
            .expect("Failed to create test context");
        assert!(ctx.temp_dir.path().exists());
        assert!(ctx.config.auth.jwt_secret.len() >= 32); // Minimum secure length
    }

    #[tokio::test]
    async fn test_create_admin_user() {
        let ctx = TestContext::new()
            .await
            .expect("Failed to create test context");
        let user_id = create_test_admin_user(&ctx.db_pool, "test@example.com", "google-123")
            .await
            .expect("Failed to create test user");

        assert!(user_id > 0);
    }
}
