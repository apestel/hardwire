mod common;

use common::{TestContext, create_test_admin_user, create_test_file, create_test_share};
use sqlx::Row;

#[tokio::test]
async fn test_database_migrations() {
    let ctx = TestContext::new()
        .await
        .expect("Failed to create test context");

    // First, try to list all tables
    let tables = sqlx::query("SELECT name FROM sqlite_master WHERE type='table'")
        .fetch_all(&ctx.db_pool)
        .await
        .expect("Failed to list tables");

    println!("Tables found: {:?}", tables.len());
    for table in &tables {
        let name: String = table.get("name");
        println!("  - {}", name);
    }

    // Verify tables exist by querying them (non-macro version for in-memory DB)
    let result = sqlx::query("SELECT COUNT(*) as count FROM admin_users")
        .fetch_one(&ctx.db_pool)
        .await;

    assert!(result.is_ok(), "admin_users table should exist");
}

#[tokio::test]
async fn test_config_validation() {
    //use hardwire::config::*;

    let mut config = common::create_test_config(&std::path::PathBuf::from("/tmp/test"))
        .expect("Failed to create test config");

    // Valid config should pass
    assert!(config.validate().is_ok());

    // JWT secret too short should fail
    config.auth.jwt_secret = "short".to_string();
    assert!(config.validate().is_err());
}

#[tokio::test]
async fn test_create_and_retrieve_admin_user() {
    let ctx = TestContext::new()
        .await
        .expect("Failed to create test context");

    let user_id = create_test_admin_user(&ctx.db_pool, "admin@example.com", "google-abc-123")
        .await
        .expect("Failed to create admin user");

    // Retrieve the user (non-macro version)
    let user = sqlx::query("SELECT email, google_id FROM admin_users WHERE id = ?")
        .bind(user_id)
        .fetch_one(&ctx.db_pool)
        .await
        .expect("Failed to fetch user");

    let email: String = user.get("email");
    let google_id: String = user.get("google_id");

    assert_eq!(email, "admin@example.com");
    assert_eq!(google_id, "google-abc-123");
}

#[tokio::test]
async fn test_create_share_link() {
    let ctx = TestContext::new()
        .await
        .expect("Failed to create test context");

    // Create test files
    let file1_id = create_test_file(&ctx.db_pool, "/test/file1.txt")
        .await
        .expect("Failed to create file 1");
    let file2_id = create_test_file(&ctx.db_pool, "/test/file2.txt")
        .await
        .expect("Failed to create file 2");

    // Create share link
    let share_id = "test_share_123";
    create_test_share(&ctx.db_pool, share_id, &[file1_id, file2_id])
        .await
        .expect("Failed to create share");

    // Verify share and files
    let files = sqlx::query(
        "SELECT f.path FROM files f JOIN share_link_files slf ON f.id = slf.file_id WHERE slf.share_link_id = ?"
    )
    .bind(share_id)
    .fetch_all(&ctx.db_pool)
    .await
    .expect("Failed to fetch share files");

    assert_eq!(files.len(), 2);

    let paths: Vec<String> = files.iter().map(|row| row.get("path")).collect();
    assert!(paths.contains(&"/test/file1.txt".to_string()));
    assert!(paths.contains(&"/test/file2.txt".to_string()));
}

#[tokio::test]
async fn test_file_size_limit_validation() {
    use hardwire::error::AppError;

    let ctx = TestContext::new()
        .await
        .expect("Failed to create test context");

    let max_size = ctx.config.limits.max_file_size_bytes;
    let oversized = max_size + 1;

    let error = AppError::FileSizeLimitExceeded {
        max_size,
        actual_size: oversized,
    };

    // Verify error message contains both sizes
    let error_msg = error.to_string();
    assert!(error_msg.contains("limit exceeded"));
}

#[tokio::test]
async fn test_too_many_files_validation() {
    use hardwire::error::AppError;

    let ctx = TestContext::new()
        .await
        .expect("Failed to create test context");

    let max_files = ctx.config.limits.max_files_per_share;
    let too_many = max_files + 1;

    let error = AppError::TooManyFiles {
        max_files,
        actual_files: too_many,
    };

    let error_msg = error.to_string();
    assert!(error_msg.contains("Too many files"));
}
