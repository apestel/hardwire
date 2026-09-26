use axum::http::header::{
    ACCEPT, ACCEPT_RANGES, AUTHORIZATION, CONTENT_LENGTH, CONTENT_RANGE, RANGE,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use url::Url;

use axum_tracing_opentelemetry::middleware::{OtelAxumLayer, OtelInResponseLayer};
use http::request::Parts as RequestParts;

// use qbittorrent::{data::Torrent, traits::TorrentData, Api};
use tokio::sync::Mutex;
use tokio::sync::broadcast;
use tokio_util::codec::{BytesCodec, FramedRead};
use tower_http::services::ServeDir;
use tracing::instrument;

use openidconnect::Nonce;

use std::collections::HashMap;

use clap::{CommandFactory, Parser};

use sqlx::{Pool, Sqlite, SqlitePool};

use tower_http::cors::{AllowOrigin, CorsLayer};

use std::fs::File;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};

use askama::Template;
use axum::body::Body;
use axum::middleware::Next;
use axum::middleware;

type Db = sqlx::SqlitePool;

use axum::extract::{ConnectInfo, Path, State};
use axum::routing::{get, head};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Mutex as StdMutex;
use std::time::Instant;

mod admin;
mod config;
mod db;
mod error;
mod file_indexer;
mod pathtools;
mod progress;
mod worker;
use config::Config;
use progress::ProgressReader;
use worker::{TaskManager, tasks::TaskWorker};

#[derive(clap::Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Server
    #[arg(short, long)]
    server: bool,

    /// Files to publish
    #[arg(short, long, num_args=1.., value_names = ["LIST OF FILES"])]
    files: Vec<String>,

    /// Initialize/migrate the database, then exit (fresh-install path, see src/db.rs)
    #[arg(long)]
    db_init: bool,
}

// AppError is now defined in the error module

/// Per-IP token bucket rate limiter (in-memory; per-process).
#[derive(Debug, Clone)]
pub struct RateLimiter {
    inner: std::sync::Arc<RateLimiterInner>,
}

#[derive(Debug)]
struct RateLimiterInner {
    limit: u32,
    buckets: StdMutex<HashMap<String, (f64, Instant)>>,
}

impl RateLimiter {
    pub fn new(requests_per_minute: u32) -> Self {
        Self {
            inner: std::sync::Arc::new(RateLimiterInner {
                limit: requests_per_minute.max(1),
                buckets: StdMutex::new(HashMap::new()),
            }),
        }
    }

    /// Consume one token for `key`; returns false when the bucket is empty.
    pub fn allow(&self, key: &str) -> bool {
        let inner = &self.inner;
        let mut buckets = match inner.buckets.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        // Opportunistic cleanup of idle buckets so the map stays bounded.
        if buckets.len() > 10_000 {
            let cutoff = Instant::now() - std::time::Duration::from_secs(300);
            buckets.retain(|_, (_, last)| *last > cutoff);
        }
        let now = Instant::now();
        let limit = inner.limit as f64;
        let (tokens, last) = buckets.entry(key.to_string()).or_insert((limit, now));
        let elapsed = now.duration_since(*last).as_secs_f64();
        *tokens = (*tokens + (limit / 60.0) * elapsed).min(limit);
        *last = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Client IP for rate limiting and download logs.
///
/// Forwarding headers are trusted only when the TCP peer is a local proxy
/// (loopback / private network, e.g. Traefik on the Docker network): a client
/// reaching the port directly cannot spoof them. Behind the proxy:
/// CF-Connecting-IP (set by Cloudflare), else the RIGHT-most X-Forwarded-For
/// entry — the one appended by the proxy; the left-most is client-controlled.
/// Header values must parse as an IP, which also bounds rate-limiter keys.
// ponytail: any local peer is trusted; if Traefik is reachable without
// Cloudflare in front, it must strip CF-Connecting-IP (or add an explicit
// trusted-proxy list here).
fn client_ip(headers: &HeaderMap, peer: IpAddr) -> String {
    let peer = peer.to_canonical();
    let behind_proxy = match peer {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local(),
    };
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    behind_proxy
        .then(|| {
            header("CF-Connecting-IP")
                .and_then(|v| v.trim().parse::<IpAddr>().ok())
                .or_else(|| {
                    header("X-Forwarded-For")
                        .and_then(|v| v.rsplit(',').next())
                        .and_then(|v| v.trim().parse::<IpAddr>().ok())
                })
        })
        .flatten()
        .unwrap_or(peer)
        .to_string()
}

/// Rate-limit middleware for public routes, keyed by [`client_ip`].
async fn rate_limit(
    State(app): State<App>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    req: axum::http::Request<Body>,
    next: Next,
) -> Response {
    let key = client_ip(req.headers(), peer_addr.ip());

    if !app.rate_limiter.allow(&key) {
        tracing::warn!(%key, "rate limit exceeded");
        return (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded. Try again later.")
            .into_response();
    }
    next.run(req).await
}

/// App holds the state of the application
#[derive(Clone, Debug)]
struct App {
    db_pool: Pool<Sqlite>,
    progress_channel_sender: broadcast::Sender<progress::Event>,
    task_manager: Arc<TaskManager>,
    indexer: file_indexer::FileIndexer,
    config: Config,
    pending_auths: Arc<Mutex<HashMap<String, (Nonce, String, i64)>>>,
    rate_limiter: Arc<RateLimiter>,
    /// Canonicalized data directory, used to refuse serving files outside of it.
    data_dir_canonical: PathBuf,
    /// Canonicalized database path — the DB file itself is never shareable,
    /// even when it lives inside the data directory.
    db_path_canonical: PathBuf,
}

impl App {
    fn new(
        pool: Pool<Sqlite>,
        progress_channel_sender: broadcast::Sender<progress::Event>,
        task_manager: Arc<TaskManager>,
        indexer: file_indexer::FileIndexer,
        config: Config,
        rate_limiter: Arc<RateLimiter>,
        data_dir_canonical: PathBuf,
        db_path_canonical: PathBuf,
    ) -> Self {
        App {
            db_pool: pool,
            progress_channel_sender,
            task_manager,
            indexer,
            config,
            pending_auths: Arc::new(Mutex::new(HashMap::new())),
            rate_limiter,
            data_dir_canonical,
            db_path_canonical,
        }
    }
}

impl App {}

async fn init_db(config: &Config) -> Db {
    let db_config = &config.database;
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&db_config.path)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(db_config.acquire_timeout_secs));

    let db = match sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(db_config.max_connections)
        .min_connections(db_config.min_connections)
        .acquire_timeout(std::time::Duration::from_secs(db_config.acquire_timeout_secs))
        .connect_with(opts)
        .await
    {
        Ok(db) => db,
        Err(e) => {
            panic!("Failed to connect to SQLx database: {}", e);
        }
    };

    // Idempotent schema bootstrap: on a brand-new database the embedded
    // runner alone can never complete (see src/db.rs), so we repair the
    // schema and the migration bookkeeping before asking it to run.
    if let Err(e) = db::bootstrap_schema(&db).await {
        panic!("Failed to initialize SQLx database schema: {}", e);
    }

    if let Err(e) = sqlx::migrate!().run(&db).await {
        panic!("Failed to initialize SQLx database: {}", e);
    }

    if let Some(email) = &config.auth.admin_email {
        if let Err(e) = db::ensure_admin(&db, email).await {
            panic!("Failed to pre-authorize admin {email}: {e}");
        }
    }
    db
}

struct ShareLink {
    link: i64,
    short_filename: String,
}

#[derive(Template)] // this will generate the code...
#[template(path = "404.html")] // using the template in this path, relative
// to the `templates` dir in the crate root
struct T404 {
    // the name of the struct can be anything
    // the field name should match the variable name
    // in your template
}

#[derive(Template)] // this will generate the code...
#[template(path = "list_files.html", print = "all")] // using the template in this path, relative
// to the `templates` dir in the crate root
struct DownloadFilesTemplate {
    // the name of the struct can be anything
    // the field name should match the variable name
    // in your template
    files: Vec<ShareLink>,
    share_id: String,
    hardwire_host: String,
    first_filename: String,
}

async fn list_shared_files(State(app_state): State<App>, Path(share_id): Path<String>) -> Response {
    let share_id_log = share_id.clone();
    let result = async move {
        let shared_links: Vec<(String, i64, String)> = sqlx::query_as(
            r#"SELECT
    files.path AS "filename!",
    files.id AS "link!",
    -- This part extracts the filename after the last '/'
    replace(files.path, rtrim(files.path, replace(files.path, '/', '')), '') AS "short_filename!"
FROM
    share_links
JOIN
    share_link_files ON share_links.id = share_link_files.share_link_id
JOIN
    files ON share_link_files.file_id = files.id
WHERE
    share_links.id = ?
    AND (share_links.expiration = -1 OR share_links.expiration > strftime('%s','now'));"#,
        )
        .bind(share_id.clone())
        .fetch_all(&app_state.db_pool)
        .await?;
        if !shared_links.is_empty() {
            let t = DownloadFilesTemplate {
                files: shared_links
                    .iter()
                    .map(|r| ShareLink {
                        link: r.1,
                        short_filename: r.2.clone(),
                    })
                    .collect(),
                share_id: share_id.to_string(),
                hardwire_host: app_state.config.server.host.clone(),
                first_filename: shared_links.first().unwrap().2.clone(),
            };

            Ok::<_, anyhow::Error>((StatusCode::OK, Html(t.render().unwrap())))
        } else {
            Ok::<_, anyhow::Error>(not_found().await)
        }
    }
    .await;

    match result {
        Ok(response) => response.into_response(),
        Err(e) => {
            // Public route: never leak internal error details to the client.
            tracing::error!(share_id = %share_id_log, "list_shared_files failed: {e}");
            not_found().await.into_response()
        }
    }
}

async fn healthcheck() -> impl IntoResponse {
    "OK"
}

/// Build metadata baked at compile time (see build.rs). Used by the
/// deployment pipeline to verify the running container matches the
/// released image, and by operators with `hardwire --version`.
const APP_VERSION: &str = env!("APP_VERSION");
const APP_GIT_SHA: &str = env!("APP_GIT_SHA");

/// `GET /version` — machine-readable build info. Public but unrate-limited:
/// it is polled by the deployment tooling after each release. Empty values
/// (local builds without build-args) fall back to "dev"/"unknown".
async fn version_info() -> axum::Json<serde_json::Value> {
    let version = if APP_VERSION.is_empty() { "dev" } else { APP_VERSION };
    let git_sha = if APP_GIT_SHA.is_empty() { "unknown" } else { APP_GIT_SHA };
    axum::Json(serde_json::json!({
        "name": "hardwire",
        "version": version,
        "git_sha": git_sha,
    }))
}

async fn head_file(
    State(app_state): State<App>,
    Path((share_id, file_id)): Path<(String, u32)>,
) -> impl IntoResponse {
    let file_path = match sqlx::query!(
        r#"SELECT path as file_path
        FROM files JOIN share_link_files ON share_link_files.file_id=files.id
        WHERE files.id=$1 AND share_link_files.share_link_id=$2"#,
        file_id,
        share_id
    )
    .fetch_one(&app_state.db_pool)
    .await
    {
        Ok(row) => row.file_path,
        Err(_) => return Err(not_found().await),
    };

    // Expired or unknown shares must behave exactly like missing ones (no oracle).
    match sqlx::query_scalar::<_, i64>("SELECT expiration FROM share_links WHERE id = ?")
        .bind(&share_id)
        .fetch_optional(&app_state.db_pool)
        .await
    {
        Ok(Some(expiration)) if expiration != -1 && expiration < chrono::Utc::now().timestamp() => {
            return Err(not_found().await)
        }
        Ok(Some(_)) => {}
        Ok(None) => return Err(not_found().await),
        Err(e) => {
            tracing::error!(share_id = %share_id, "failed to check share expiration: {e}");
            return Err(not_found().await);
        }
    }

    // Defense in depth: never serve a file that resolves outside the data directory.
    if let Ok(canonical) = tokio::fs::canonicalize(&file_path).await {
        if !canonical.starts_with(&app_state.data_dir_canonical) {
            tracing::warn!(path = %file_path, "refusing to serve file outside the data directory");
            return Err(not_found().await);
        }
        if canonical == app_state.db_path_canonical {
            tracing::warn!(path = %file_path, "refusing to serve the database file");
            return Err(not_found().await);
        }
    }

    let file_size = match tokio::fs::metadata(&file_path).await {
        Ok(m) => m.len(),
        Err(_) => return Err(not_found().await),
    };

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_LENGTH, file_size.to_string().parse().unwrap());
    Ok(headers)
}

#[instrument(skip(app_state))]
async fn download_file(
    State(app_state): State<App>,
    Path((share_id, file_id)): Path<(String, u32)>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ip_address = client_ip(&headers, peer_addr.ip());
    let file_path = match sqlx::query!(
        r#"SELECT path as file_path
    FROM files JOIN share_link_files ON share_link_files.file_id=files.id
    WHERE files.id=$1 AND share_link_files.share_link_id=$2"#,
        file_id,
        share_id
    )
    .fetch_one(&app_state.db_pool)
    .await
    {
        Ok(row) => row.file_path,
        Err(_) => return Err(not_found().await),
    };

    // Expired or unknown shares must behave exactly like missing ones (no oracle).
    match sqlx::query_scalar::<_, i64>("SELECT expiration FROM share_links WHERE id = ?")
        .bind(&share_id)
        .fetch_optional(&app_state.db_pool)
        .await
    {
        Ok(Some(expiration)) if expiration != -1 && expiration < chrono::Utc::now().timestamp() => {
            return Err(not_found().await)
        }
        Ok(Some(_)) => {}
        Ok(None) => return Err(not_found().await),
        Err(e) => {
            tracing::error!(share_id = %share_id, "failed to check share expiration: {e}");
            return Err(not_found().await);
        }
    }

    let mut file = match tokio::fs::File::open(&file_path).await {
        Ok(file) => file,
        Err(_) => return Err(not_found().await),
    };
    // Defense in depth: never serve a file that resolves outside the data directory.
    if let Ok(canonical) = tokio::fs::canonicalize(&file_path).await {
        if !canonical.starts_with(&app_state.data_dir_canonical) {
            tracing::warn!(path = %file_path, "refusing to serve file outside the data directory");
            return Err(not_found().await);
        }
        if canonical == app_state.db_path_canonical {
            tracing::warn!(path = %file_path, "refusing to serve the database file");
            return Err(not_found().await);
        }
    }
    // fstat on the open file: always the real size (an indexer cache could be
    // minutes stale and produce a wrong Content-Length).
    let file_size = match file.metadata().await {
        Ok(m) => m.len(),
        Err(e) => {
            tracing::error!("failed to read file metadata: {e}");
            return Err(not_found().await);
        }
    };
    // Unique ID per download request so each download gets its own tracking entry
    let transaction_id = uuid::Uuid::new_v4().to_string();

    // Handle range request
    let (start, end) = if let Some(range) = headers.get(RANGE) {
        if let Ok(range_str) = range.to_str() {
            if let Some(range_val) = range_str.strip_prefix("bytes=") {
                let ranges: Vec<&str> = range_val.split('-').collect();
                if ranges.len() == 2 {
                    let start = ranges[0].parse::<u64>().unwrap_or(0);
                    let end = ranges[1]
                        .parse::<u64>()
                        .unwrap_or(file_size - 1)
                        .min(file_size - 1);
                    if start <= end {
                        (start, end)
                    } else {
                        (0, file_size - 1)
                    }
                } else {
                    (0, file_size - 1)
                }
            } else {
                (0, file_size - 1)
            }
        } else {
            (0, file_size - 1)
        }
    } else {
        (0, file_size - 1)
    };

    // Seek to the start position if it's not 0
    if start > 0 {
        use tokio::io::AsyncSeekExt;
        if let Err(e) = file.seek(std::io::SeekFrom::Start(start)).await {
            tracing::error!("failed to seek download stream: {e}");
            return Ok((StatusCode::INTERNAL_SERVER_ERROR, "Internal server error").into_response());
        }
    }

    let content_length = end - start + 1;
    let progress_reader = ProgressReader::new(
        file,
        content_length as u32,
        file_size,
        transaction_id,
        file_path,
        ip_address,
        app_state.progress_channel_sender,
        start,
    );
    let frame_reader = FramedRead::new(progress_reader, BytesCodec::new());
    // let body_stream = http_body_util::BodyStream::new(frame_reader);
    let body = Body::from_stream(frame_reader);

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_LENGTH, content_length.to_string().parse().unwrap());

    if start != 0 || end != file_size - 1 {
        headers.insert(
            CONTENT_RANGE,
            format!("bytes {}-{}/{}", start, end, file_size)
                .parse()
                .unwrap(),
        );
        headers.insert(ACCEPT_RANGES, "bytes".parse().unwrap());
        Ok((StatusCode::PARTIAL_CONTENT, headers, body).into_response())
    } else {
        headers.insert(ACCEPT_RANGES, "bytes".parse().unwrap());
        Ok((headers, body).into_response())
    }
}

async fn publish_files(
    files: Vec<String>,
    base_url: &String,
    db_pool: &SqlitePool,
) -> Result<String> {
    let share_id = nanoid::nanoid!(10);
    let now = chrono::offset::Utc::now().timestamp();

    let mut tx = db_pool.begin().await?;

    let mut files_id: Vec<i64> = vec![];
    for filename in files {
        if std::path::Path::new(&filename).exists() {
            let file = File::open(&filename)?;
            let file_size = i64::try_from(file.metadata()?.len())?;
            let row = sqlx::query!(
                "INSERT INTO files (sha256, path, file_size) VALUES ($1, $2, $3)",
                "",
                filename,
                file_size
            )
            .execute(&mut *tx)
            .await
            .map_err(|e| anyhow!("failed to insert file: {:?}", e))?;
            files_id.push(row.last_insert_rowid());
        }
    }

    if files_id.is_empty() {
        return Err(anyhow::Error::msg("no valid files to share"));
    }

    sqlx::query!(
        "INSERT INTO share_links (id, expiration, created_at) VALUES ($1, $2, $3)",
        share_id,
        -1,
        now
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| anyhow!("failed to create share link: {:?}", e))?;

    for id in &files_id {
        sqlx::query!(
            "INSERT INTO share_link_files (share_link_id, file_id) VALUES ($1, $2)",
            share_id,
            id
        )
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    Ok(format!("{}/s/{}", base_url, share_id))
}

// ServerConfig is now defined in the config module

async fn not_found() -> (StatusCode, Html<String>) {
    let t = T404 {};
    (StatusCode::NOT_FOUND, Html(t.render().unwrap()))
}

#[tokio::main]
async fn main() -> Result<()> {
    pretty_env_logger::init();

    let cli = Cli::parse();

    // Load and validate configuration
    let config = Config::from_env().context("Failed to load configuration")?;
    config
        .validate()
        .context("Configuration validation failed")?;

    // `--db-init`: initialize/migrate the database, then exit. Used by CI and
    // operators on fresh installs.
    if cli.db_init {
        let _db = init_db(&config).await;
        println!("database ready: {}", config.database.path.display());
        return Ok(());
    }

    let db_pool = init_db(&config).await;

    if cli.files.is_empty() && !cli.server {
        // let out = std::io::stdout();
        Cli::command().print_long_help()?;
    }

    if !cli.files.is_empty() {
        let shared_link = publish_files(cli.files, &config.server.host, &db_pool).await?;
        println!("Shared link: {}", shared_link);
    }

    if cli.server {
        let _guard = init_tracing_opentelemetry::TracingConfig::production().init_subscriber()?;
        let mut progress_manager = progress::Manager::new(db_pool.clone());
        // let base_path = "/mnt";
        let indexer = file_indexer::FileIndexer::new(
            &config.server.data_dir,
            config.limits.file_indexer_interval_secs,
        );

        let progress_channel_sender = progress_manager.sender.clone();
        progress_manager.start_recv_thread().await;

        // Initialize task manager
        let (task_manager, task_receiver) = TaskManager::new(db_pool.clone());
        let task_manager = Arc::new(task_manager);

        // Start task worker
        let worker_task_manager = Arc::clone(&task_manager);
        let worker_data_dir = config.server.data_dir.clone();
        tokio::spawn(async move {
            let mut worker = TaskWorker::new((*worker_task_manager).clone(), task_receiver, worker_data_dir);
            worker.run().await;
        });

        let rate_limiter = Arc::new(RateLimiter::new(
            config.limits.rate_limit_requests_per_minute,
        ));
        let data_dir_canonical = std::fs::canonicalize(&config.server.data_dir)
            .unwrap_or_else(|_| config.server.data_dir.clone());
        let db_path_canonical = std::fs::canonicalize(&config.database.path)
            .unwrap_or_else(|_| config.database.path.clone());

        let app_state = App::new(
            db_pool,
            progress_channel_sender,
            task_manager,
            indexer,
            config.clone(),
            rate_limiter,
            data_dir_canonical,
            db_path_canonical,
        );

        // Public routes are rate-limited; admin routes are JWT-protected instead.
        let public_router = axum::Router::new()
            .route("/s/{share_id}", get(list_shared_files))
            .route(
                "/s/{share_id}/{file_id}",
                head(head_file).get(download_file),
            )
            .route("/healthcheck", get(healthcheck))
            .layer(middleware::from_fn_with_state(app_state.clone(), rate_limit))
            .with_state(app_state.clone());

        let app = axum::Router::new()
            .route("/version", get(version_info))
            .merge(public_router)
            .nest_service("/assets", ServeDir::new("dist/"))
            .nest("/admin", admin::admin_router())
            .with_state(app_state)
            // include trace context as header into the response
            .layer(OtelInResponseLayer)
            //start OpenTelemetry trace on incoming request
            .layer(OtelAxumLayer::default())
            .layer(
                CorsLayer::new()
                    .allow_origin(AllowOrigin::predicate(
                        |origin: &HeaderValue, _request_parts: &RequestParts| {
                            origin.as_bytes().ends_with(b".pestel.me")
                                || match Url::parse(std::str::from_utf8(origin.as_ref()).unwrap()) {
                                    Ok(url) => url.host_str().unwrap().eq("localhost"),
                                    Err(_) => false,
                                }
                        },
                    ))
                    .allow_headers([AUTHORIZATION, ACCEPT])
                    .allow_credentials(true),
            );

        let bind_adress = format!("0.0.0.0:{}", config.server.port);
        let listener = tokio::net::TcpListener::bind(bind_adress).await.unwrap();
        axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(shutdown_signal())
            .await
            .unwrap();
    }
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::warn!("signal received, starting graceful shutdown");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().unwrap());
        }
        h
    }

    #[test]
    fn client_ip_ignores_headers_from_public_peers() {
        let h = headers(&[("CF-Connecting-IP", "1.1.1.1"), ("X-Forwarded-For", "2.2.2.2")]);
        assert_eq!(client_ip(&h, "8.8.8.8".parse().unwrap()), "8.8.8.8");
    }

    #[test]
    fn client_ip_behind_proxy() {
        let proxy: IpAddr = "172.18.0.2".parse().unwrap();
        // Right-most XFF entry is the one the proxy appended.
        let h = headers(&[("X-Forwarded-For", "6.6.6.6, 9.9.9.9")]);
        assert_eq!(client_ip(&h, proxy), "9.9.9.9");
        // Cloudflare header wins.
        let h = headers(&[("CF-Connecting-IP", "1.1.1.1"), ("X-Forwarded-For", "9.9.9.9")]);
        assert_eq!(client_ip(&h, proxy), "1.1.1.1");
        // Garbage falls back to the peer; IPv4-mapped peers count as private.
        let h = headers(&[("X-Forwarded-For", "not-an-ip")]);
        assert_eq!(client_ip(&h, "::ffff:10.0.0.1".parse().unwrap()), "10.0.0.1");
    }
}
