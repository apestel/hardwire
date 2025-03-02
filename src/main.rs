use axum::http::header::{ACCEPT, ACCEPT_RANGES, AUTHORIZATION, CONTENT_LENGTH, CONTENT_RANGE, RANGE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use url::Url;

use axum_tracing_opentelemetry::middleware::{OtelAxumLayer, OtelInResponseLayer};
use http::request::Parts as RequestParts;

// use qbittorrent::{data::Torrent, traits::TorrentData, Api};
use tokio::sync::broadcast;
use tokio_util::codec::{BytesCodec, FramedRead};
use tower_http::services::ServeDir;
use tracing::instrument;

use clap::{CommandFactory, Parser};

use sqlx::{Pool, Sqlite, SqlitePool};

use tower_http::cors::{AllowOrigin, CorsLayer};

use std::fs::File;
use std::sync::Arc;

use anyhow::{anyhow,Result};
use std::env;
use std::path::PathBuf;

use askama::Template;
use axum::body::Body;

extern crate chrono;

type Db = sqlx::SqlitePool;

use axum::routing::{get, head};
use axum::extract::{ Path, State};


mod file_indexer;
mod progress;
mod worker;
mod admin;
use progress::ProgressReader;
use tracing_opentelemetry_instrumentation_sdk::find_current_trace_id;
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

// Make our own error that wraps `anyhow::Error`.
struct AppError(anyhow::Error);

// Tell axum how to convert `AppError` into a response.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Something went wrong: {}", self.0),
        )
            .into_response()
    }
}

// This enables using `?` on functions that return `Result<_, anyhow::Error>` to turn them into
// `Result<_, AppError>`. That way you don't need to do that manually.
impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

/// App holds the state of the application
#[derive(Clone, Debug)]
struct App {
    db_pool: Pool<Sqlite>,
    progress_channel_sender: broadcast::Sender<progress::Event>,
    task_manager: Arc<TaskManager>,
    indexer: file_indexer::FileIndexer,
    server_config: ServerConfig,
}

impl App {
    fn new(
        pool: Pool<Sqlite>,
        progress_channel_sender: broadcast::Sender<progress::Event>,
        task_manager: Arc<TaskManager>,
        indexer: file_indexer::FileIndexer,
        server_config: ServerConfig
    ) -> Self {
        App {
            db_pool: pool,
            progress_channel_sender,
            task_manager,
            indexer,
            server_config,
        }
    }
}

impl App {}

async fn init_db(data_dir: PathBuf) -> Db {
    let mut sqlite_path = data_dir.clone();
    sqlite_path.push("db.sqlite");

    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(sqlite_path)
        .create_if_missing(true);

    // opts.disable_statement_logging();
    match Db::connect_with(opts).await {
        Ok(db) => db,
        Err(e) => {
            panic!("Failed to connect to SQLx database: {}", e);
        }
    } 
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

async fn list_shared_files(
    State(app_state): State<App>,
    Path(share_id): Path<String>,
) -> Response {
    let result = async move {
        let shared_links: Vec<(String, i64, String)> = sqlx::query_as(
            r#"SELECT files.path AS "filename!", files.id AS "link!", substr(files.path, instr(files.path, '/') + 1) AS "short_filename!"
        FROM share_links JOIN share_link_files ON share_links.id=share_link_files.share_link_id
        JOIN files ON share_link_files.file_id=files.id
        WHERE share_links.id = ?"#
        )
        .bind(share_id.clone())
        .fetch_all(&app_state.db_pool)
        .await?;
        let server = ServerConfig::new();
        
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
                hardwire_host: server.host,
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
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Something went wrong: {}", e)).into_response(),
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

    let file = match tokio::fs::File::open(file_path.clone()).await {
        Ok(file) => file,
        Err(_) => return Err(not_found().await),
    };
    let file_size = file.metadata().await.unwrap().len();

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_LENGTH, file_size.to_string().parse().unwrap());
    Ok(headers)
}

#[instrument(skip(app_state))]
async fn download_file(
    State(app_state): State<App>,
    Path((share_id, file_id)): Path<(String, u32)>,
    headers: HeaderMap,
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

    let mut file = match tokio::fs::File::open(file_path.clone()).await {
        Ok(file) => file,
        Err(_) => return Err(not_found().await),
    };
    let file_size = file.metadata().await.unwrap().len();
    let transaction_id = find_current_trace_id().unwrap();

    // Handle range request
    let (start, end) = if let Some(range) = headers.get(RANGE) {
        if let Ok(range_str) = range.to_str() {
            if let Some(range_val) = range_str.strip_prefix("bytes=") {
                let ranges: Vec<&str> = range_val.split('-').collect();
                if ranges.len() == 2 {
                    let start = ranges[0].parse::<u64>().unwrap_or(0);
                    let end = ranges[1].parse::<u64>().unwrap_or(file_size - 1).min(file_size - 1);
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
            return Ok((StatusCode::INTERNAL_SERVER_ERROR, format!("Something went wrong: {}", e)).into_response());
        }
    }

    let content_length = end - start + 1;
    let progress_reader = ProgressReader::new(
        file,
        content_length as u32,
        transaction_id,
        file_path,
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
            format!("bytes {}-{}/{}", start, end, file_size).parse().unwrap(),
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
    let mut files_id: Vec<i64> = vec![];
    let share_id = nanoid::nanoid!(10);

    for filename in files {
        if std::path::Path::new(&filename).exists() {
            let file = File::open(&filename)?;
            let file_size = i64::try_from(file.metadata().unwrap().len()).unwrap();
            // FIXME: Should implement a SQL Transaction with BEGIN/ROLLBACK in case of error
            match sqlx::query!(
                "INSERT INTO files (sha256, path, file_size) VALUES ($1, $2, $3)",
                "",
                filename,
                file_size
            )
            .execute(db_pool)
            .await
            {
                Ok(row) => files_id.push(row.last_insert_rowid()),
                Err(e) => return Err(anyhow!("failed to create share link: {:?}", e)),
            };
        }
    }
    if !files_id.is_empty() {
        let now = chrono::offset::Utc::now().timestamp();
        match sqlx::query!(
            "INSERT INTO share_links (id, expiration, created_at) VALUES ($1, $2, $3)",
            share_id,
            -1,
            now
        )
        .execute(db_pool)
        .await
        {
            Ok(_) => {
                for id in files_id {
                    sqlx::query!(
                        "INSERT INTO share_link_files (share_link_id, file_id) VALUES ($1, $2)",
                        share_id,
                        id
                    )
                    .execute(db_pool)
                    .await?;
                }
                return Ok(format!("{}/s/{}", base_url, share_id));
            }
            Err(e) => {
                log::error!("{}", e);
                return Err(anyhow!("failed to create share link: {:?}", e));
            }
        };
    }
    Err(anyhow::Error::msg("failed to create share link"))
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub port: u16,
    pub base_path: String,
    pub host: String,
    pub data_dir: Arc<PathBuf>,
}

impl ServerConfig {
    const STD_PORT: u16 = 8090;
    const STD_BASE_PATH: &'static str = ".";
    const STD_HOST: &'static str = "http://localhost:8090";
    const PORT_ENV_VAR: &'static str = "HARDWIRE_PORT";
    const BASE_PATH_ENV_VAR: &'static str = "HARDWIRE_BASE_PATH";
    const HOST_ENV_VAR: &'static str = "HARDWIRE_HOST";
    const STD_HARDWIRE_DATA_DIR: &'static str = ".";
    const HARDWIRE_DATA_DIR_ENV_VAR: &'static str = "HARDWIRE_DATA_DIR";

    fn new() -> ServerConfig {
        ServerConfig {
            port: Self::port_from_env(),
            base_path: Self::base_path_from_env(),
            host: Self::host_from_env(),
            data_dir: Arc::new(Self::data_dir_from_env()),
        }
    }

    fn port_from_env() -> u16 {
        // Also shortened the `match` a bit here. Could make this generic too.
        env::var(ServerConfig::PORT_ENV_VAR)
            .map(|val| val.parse::<u16>())
            .unwrap_or(Ok(ServerConfig::STD_PORT))
            .unwrap()
    }

    fn base_path_from_env() -> String {
        env::var(ServerConfig::BASE_PATH_ENV_VAR).unwrap_or(ServerConfig::STD_BASE_PATH.to_string())
    }

    fn host_from_env() -> String {
        env::var(ServerConfig::HOST_ENV_VAR).unwrap_or(ServerConfig::STD_HOST.to_string())
    }

    fn data_dir_from_env() -> PathBuf {
        PathBuf::from(
            env::var(ServerConfig::HARDWIRE_DATA_DIR_ENV_VAR)
                .unwrap_or(ServerConfig::STD_HARDWIRE_DATA_DIR.to_string()),
        )
    }
}

async fn not_found() -> (StatusCode, Html<String>) {
    let t = T404 {};
    (StatusCode::NOT_FOUND, Html(t.render().unwrap()))
}



#[tokio::main]
async fn main() -> Result<()> {
    pretty_env_logger::init();

    let cli = Cli::parse();
    let server_config = ServerConfig::new();
    let db_pool = init_db(server_config.data_dir.to_path_buf()).await;

    if cli.files.is_empty() && !cli.server {
        // let out = std::io::stdout();
        Cli::command().print_long_help()?;
    }

    if !cli.files.is_empty() {
        let shared_link = publish_files(cli.files, &server_config.host, &db_pool).await?;
        println!("Shared link: {}", shared_link);
    }

    if cli.server {
        let _ = init_tracing_opentelemetry::tracing_subscriber_ext::init_subscribers()?;
        let mut progress_manager = progress::Manager::new(db_pool.clone());
        // let base_path = "/mnt";
        let indexer =
            file_indexer::FileIndexer::new(&PathBuf::from(&server_config.base_path.as_str()), 60);

        let progress_channel_sender = progress_manager.sender.clone();
        progress_manager.start_recv_thread().await;

        // Initialize task manager
        let (task_manager, task_receiver) = TaskManager::new(db_pool.clone());
        let task_manager = Arc::new(task_manager);
        
        // Start task worker
        let worker_task_manager = Arc::clone(&task_manager);
        tokio::spawn(async move {
            let mut worker = TaskWorker::new((*worker_task_manager).clone(), task_receiver);
            worker.run().await;
        });

        let server_config_clone = server_config.clone();
        let app_state = App::new(db_pool, progress_channel_sender, task_manager, indexer, server_config_clone);

        let app = axum::Router::new()
            .route("/s/{share_id}", get(list_shared_files))
            .route("/s/{share_id}/{file_id}", head(head_file).get(download_file))
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

        let bind_adress = format!("0.0.0.0:{}", server_config.port);
        let listener = tokio::net::TcpListener::bind(bind_adress).await.unwrap();
        axum::serve(listener, app)
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
    opentelemetry::global::shutdown_tracer_provider();
}
