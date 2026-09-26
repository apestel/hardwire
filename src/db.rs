//! Fresh-database bootstrap for the embedded migration runner.
//!
//! Why this exists: sqlx orders migrations by **numeric** version, so the
//! 12-digit version `202201011537` (the file that creates `share_links`,
//! `share_link_files`, `files` and `download`) sorts after every 8-digit
//! 2025/2026 migration and runs **last**. On a brand-new database the
//! runner therefore dies in `20250319`, which creates an index on
//! `download` before that table has been created — fresh Docker images, CI
//! and new operator installs all panic at boot (and any database left
//! partially migrated by such a boot is stuck).
//!
//! `bootstrap_schema` repairs that with the minimal possible footprint:
//!
//! 1. it pre-creates the four base tables of `202201011537` with
//!    `IF NOT EXISTS` (no-op on healthy databases), so the 2025/2026
//!    migrations that reference them succeed in their (correct) order;
//! 2. it records `202201011537` as already applied in `_sqlx_migrations`
//!    with its real checksum, so the runner skips it — its plain
//!    `CREATE TABLE` statements would otherwise collide with the tables
//!    from step 1. On databases where the migration ran long ago the row
//!    already exists and this is a no-op.
//! 3. it records `20250302` (which seeds the author's own Google account as
//!    admin) as applied too, so fresh installs of the public image never get
//!    that account. Existing deployments already ran it: no-op there. The
//!    first admin of a fresh install comes from `HARDWIRE_ADMIN_EMAIL`
//!    (see [`ensure_admin`]).
//!
//! Every other migration still runs for real through the normal runner, so
//! the migration files remain the single source of truth (no schema
//! duplication to drift). No historical migration file is modified, so the
//! checksums recorded on existing deployments stay valid.

use sqlx::{Error as SqlxError, SqlitePool};

/// The badly-versioned base-tables migration (12-digit version sorts after
/// the 8-digit 2025/2026 ones).
const BASE_TABLES_MIGRATION_VERSION: i64 = 202201011537;

/// Migration that hardcodes the author's Google account as admin.
const SEED_ADMIN_MIGRATION_VERSION: i64 = 20250302;

/// Exact table definitions of `202201011537_create-share-tables.sql`, made
/// idempotent.
const BASE_TABLES_DDL: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS share_links (id TEXT PRIMARY KEY NOT NULL, expiration INT NOT NULL, created_at INT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS share_link_files (share_link_id TEXT, file_id INT)",
    "CREATE TABLE IF NOT EXISTS files (id INTEGER PRIMARY KEY AUTOINCREMENT, info TEXT, file_size BIGINT, sha256 TEXT, path TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS download (id INTEGER PRIMARY KEY AUTOINCREMENT, file_path TEXT, ip_address TEXT, transaction_id TEXT, status TEXT, file_size INT, started_at INT, finished_at INT)",
];

/// Repair the schema/bookkeeping of fresh or partially migrated databases so
/// `sqlx::migrate!().run()` can complete. No-op on healthy databases.
pub async fn bootstrap_schema(db: &SqlitePool) -> Result<(), SqlxError> {
    // 1) Make the base tables available before the 2025/2026 migrations run.
    for stmt in BASE_TABLES_DDL {
        sqlx::query(*stmt).execute(db).await?;
    }

    // 2) + 3) Record the base-tables and seed-admin migrations as applied so
    //    the runner skips them (see module docs).
    // Note: column list mirrors the bookkeeping table that sqlx 0.9 creates
    // (`checksum BLOB NOT NULL`). On databases created under sqlx 0.8 the table
    // already exists (with a nullable `checksum`), so this is a no-op there and
    // the runner keeps working (it always writes a non-null checksum).
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _sqlx_migrations (
            version BIGINT PRIMARY KEY,
            description TEXT NOT NULL,
            installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            success BOOLEAN NOT NULL,
            checksum BLOB NOT NULL,
            execution_time BIGINT NOT NULL
        )",
    )
    .execute(db)
    .await?;

    let migrator = sqlx::migrate!();
    let installed_on = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    for version in [BASE_TABLES_MIGRATION_VERSION, SEED_ADMIN_MIGRATION_VERSION] {
        let migration = migrator
            .iter()
            .find(|m| m.version == version)
            .unwrap_or_else(|| panic!("built-in migration {version} must be present"));
        sqlx::query(
            "INSERT OR IGNORE INTO _sqlx_migrations \
             (version, description, installed_on, success, checksum, execution_time) \
             VALUES (?, ?, ?, 1, ?, 0)",
        )
        .bind(migration.version)
        .bind(migration.description.as_ref())
        .bind(&installed_on)
        .bind(migration.checksum.to_vec())
        .execute(db)
        .await?;
    }

    Ok(())
}

/// Pre-authorize `email` as admin (idempotent). It becomes a real admin on its
/// first Google login, which links the Google ID to the row.
pub async fn ensure_admin(db: &SqlitePool, email: &str) -> Result<(), SqlxError> {
    sqlx::query("INSERT OR IGNORE INTO admin_users (email, created_at) VALUES (?, ?)")
        .bind(email)
        .bind(chrono::Utc::now().timestamp())
        .execute(db)
        .await?;
    Ok(())
}
