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

use openidconnect::{Nonce, PkceCodeVerifier};
use openidconnect::core::CoreProviderMetadata;

use std::collections::HashMap;

use clap::{CommandFactory, Parser};

use sqlx::{Pool, Sqlite, SqlitePool};

use tower_http::cors::{AllowOrigin, CorsLayer};

use std::fs::File;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};

use askama::Template;
use axum::body::Body;

type Db = sqlx::SqlitePool;

use axum::extract::{ConnectInfo, Path, State};
use axum::routing::{get, head};
use std::net::SocketAddr;

mod admin;
mod config;
mod error;
mod file_indexer;
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
}

// AppError is now defined in the error module

/// App holds the state of the application
#[derive(Clone, Debug)]
struct App {
    db_pool: Pool<Sqlite>,
    progress_channel_sender: broadcast::Sender<progress::Event>,
    task_manager: Arc<TaskManager>,
    indexer: file_indexer::FileIndexer,
    config: Config,
    pending_auths: Arc<Mutex<HashMap<String, (Nonce, PkceCodeVerifier, i64)>>>,
    oidc_metadata: Arc<tokio::sync::OnceCell<CoreProviderMetadata>>,
}

impl App {
    fn new(
        pool: Pool<Sqlite>,
        progress_channel_sender: broadcast::Sender<progress::Event>,
        task_manager: Arc<TaskManager>,
        indexer: file_indexer::FileIndexer,
        config: Config,
    ) -> Self {
        App {
            db_pool: pool,
            progress_channel_sender,
            task_manager,
            indexer,
            config,
            pending_auths: Arc::new(Mutex::new(HashMap::new())),
            oidc_metadata: Arc::new(tokio::sync::OnceCell::new()),
        }
    }
}

impl App {}

async fn init_db(db_config: &config::DatabaseConfig) -> Db {
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

    if let Err(e) = sqlx::migrate!().run(&db).await {
        panic!("Failed to initialize SQLx database: {}", e);
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
    share_links.id = ?;"#,
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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Something went wrong: {}", e),
        )
            .into_response(),
    }
}

async fn healthcheck() -> impl IntoResponse {
    "OK"
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

    // Try the in-memory indexer cache before opening the file just for metadata.
    let file_size = if let Some(size) = app_state.indexer.get_file_size(&file_path) {
        size
    } else {
        let file = match tokio::fs::File::open(&file_path).await {
            Ok(file) => file,
            Err(_) => return Err(not_found().await),
        };
        match file.metadata().await {
            Ok(m) => m.len(),
            Err(_) => return Err(not_found().await),
        }
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
    // Priority: CF-Connecting-IP (Cloudflare) > X-Forwarded-For (Traefik) > direct peer
    let ip_address = headers
        .get("CF-Connecting-IP")
        .or_else(|| headers.get("X-Forwarded-For"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
        .unwrap_or_else(|| peer_addr.ip().to_string());
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

    let mut file = match tokio::fs::File::open(&file_path).await {
        Ok(file) => file,
        Err(_) => return Err(not_found().await),
    };
    // Try the in-memory indexer cache before calling fstat on the open file.
    let file_size = if let Some(size) = app_state.indexer.get_file_size(&file_path) {
        size
    } else {
        match file.metadata().await {
            Ok(m) => m.len(),
            Err(e) => return Ok((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read file metadata: {}", e),
            ).into_response()),
        }
    };
    // Stable ID across range requests: same client downloading same file in same share
    let transaction_id = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(format!("{}:{}:{}", share_id, file_id, ip_address));
        format!("{:x}", h.finalize())[..16].to_string()
    };

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
            return Ok((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Something went wrong: {}", e),
            )
                .into_response());
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
    let db_pool = init_db(&config.database).await;

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

        let app_state = App::new(
            db_pool,
            progress_channel_sender,
            task_manager,
            indexer,
            config.clone(),
        );

        let app = axum::Router::new()
            .route("/s/{share_id}", get(list_shared_files))
            .route(
                "/s/{share_id}/{file_id}",
                head(head_file).get(download_file),
            )
            .route("/healthcheck", get(healthcheck))
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
