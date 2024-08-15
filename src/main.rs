use axum::extract::ws::WebSocket;

use axum::http::header::{ACCEPT, AUTHORIZATION, CONTENT_LENGTH};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::Json;

use url::Url;

use axum_tracing_opentelemetry::middleware::{OtelAxumLayer, OtelInResponseLayer};
use file_indexer::FileInfo;
use http::request::Parts as RequestParts;

// use qbittorrent::{data::Torrent, traits::TorrentData, Api};
use tokio::sync::broadcast;
use tokio_util::codec::{BytesCodec, FramedRead};
use tower_http::services::ServeDir;
use tracing::instrument;

use clap::{CommandFactory, Parser};
use log::info;

use sqlx::{Pool, Sqlite, SqlitePool};

use tower_http::cors::{AllowOrigin, CorsLayer};

use std::fs::File;

use anyhow::Result;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::{env, ffi::OsStr};

use matroska::Matroska;

use askama::Template;
use axum::body::Body;

extern crate chrono;

type Db = sqlx::SqlitePool;

use axum::extract::{ConnectInfo, Path, State, WebSocketUpgrade};
use axum::routing::{get, post};

mod file_indexer;
mod progress;
use progress::ProgressReader;
use tracing_opentelemetry_instrumentation_sdk::find_current_trace_id;

#[derive(clap::Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Server
    #[arg(short, long)]
    server: bool,

    /// Files to publish
    #[arg(short, long, num_args=1..=99, value_delimiter = ' ', value_names = ["LIST OF FILES"])]
    files: Vec<String>,
}

/// App holds the state of the application
#[derive(Clone, Debug)]
struct App {
    //file_list: Vec<(String, Option<Torrent>)>,
    db_pool: Pool<Sqlite>,
    progress_channel_sender: broadcast::Sender<progress::Event>,
    indexer: file_indexer::FileIndexer,
}

impl App {
    fn new(
        pool: Pool<Sqlite>,
        progress_channel_sender: broadcast::Sender<progress::Event>,
        indexer: file_indexer::FileIndexer,
    ) -> Self {
        App {
            //  file_list: Vec::new(),
            db_pool: pool,
            progress_channel_sender,
            indexer,
        }
    }
}

// struct DownloadLink {
//     id: String,
//     filename: String,
//     file_sha256: String,
//     expiration: i64,
//     created_at: i64,
// }

// struct Downloads {
//     file_sha256: String,
//     download_count: u32,
//     src_ip_address: String,
// }

// struct TorrentInfoSimple {
//     name: String,
//     size: u64,
// }

impl App {}

async fn init_db(data_dir: PathBuf) -> Db {
    let mut sqlite_path = data_dir.clone();
    sqlite_path.push("db.sqlite");

    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(sqlite_path)
        .create_if_missing(true);

    // opts.disable_statement_logging();
    let db = match Db::connect_with(opts).await {
        Ok(db) => db,
        Err(e) => {
            panic!("Failed to connect to SQLx database: {}", e);
        }
    };

    if let Err(e) = sqlx::migrate!("db/migrations").run(&db).await {
        panic!("Failed to initialize SQLx database: {}", e);
    }
    db
}

#[derive(Debug)]
struct ShareLink {
    short_filename: String,
    filename: String,
    link: i64,
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

fn short_filename(filename: String) -> String {
    let s: Vec<&str> = filename.split('/').collect();
    if !s.is_empty() {
        return s[s.len() - 1].to_string();
    }
    s[0].to_string()
}

async fn list_shared_files(
    State(app_state): State<App>,
    Path(share_id): Path<String>,
) -> impl IntoResponse {
    //let conn = db_pool.get().expect("couldn't get db connection from pool");

    // let share_id = req.match_info().get("share_id").unwrap();
    // let real_peer_addr = req.peer_addr().unwrap().ip().to_string();
    // let real_peer_addr = req.connection_info().realip_remote_addr().unwrap();
    match sqlx::query_as!(
        ShareLink,
        r#"SELECT files.path AS filename, files.id AS link, '' AS short_filename
    FROM share_links JOIN share_link_files ON share_links.id=share_link_files.share_link_id
    JOIN files ON share_link_files.file_id=files.id
    WHERE share_links.id=$1"#,
        share_id
    )
    .fetch_all(&app_state.db_pool)
    .await
    {
        Ok(mut rows) => {
            let server = ServerConfig::new();
            if !rows.is_empty() {
                for r in rows.iter_mut() {
                    r.short_filename = short_filename(r.filename.clone());
                }
                let first_filename: String = rows.first().unwrap().short_filename.clone();

                let t = DownloadFilesTemplate {
                    files: rows,
                    share_id: share_id.to_string(),
                    hardwire_host: server.host,
                    first_filename,
                };

                (StatusCode::OK, Html(t.render().unwrap()))
            } else {
                not_found().await
            }
        }
        Err(_) => (StatusCode::BAD_REQUEST, Html("".to_string())),
    }
    //HttpResponse::Ok().body(real_peer_addr)
    // println!("IP address: {}", peer_addr);
    // let r = match sqlx::query_as!().await {
    //     Ok(row) => ,
    //     Err(e) =>
    // };
}

async fn healthcheck() -> impl IntoResponse {
    "OK"
}

#[instrument(skip(app_state))]
async fn download_file(
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
    let transaction_id = find_current_trace_id().unwrap();

    let progress_reader = ProgressReader::new(
        file,
        file_size as u32,
        transaction_id,
        file_path,
        app_state.progress_channel_sender,
    );
    let frame_reader = FramedRead::new(progress_reader, BytesCodec::new());
    // let body_stream = http_body_util::BodyStream::new(frame_reader);
    let body = Body::from_stream(frame_reader);

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(CONTENT_LENGTH, file_size.to_string().parse().unwrap());
    Ok((headers, body))

    //Ok((headers, body))
}

fn get_matroska_info(filename: &String) -> std::io::Result<()> {
    let file = File::open(filename)?;
    let matroska = Matroska::open(file).unwrap();
    let mut i = 0;
    for t in matroska.video_tracks() {
        i += 1;
        println!("Video Track N°{}: {:#?}", &i, &t);
        println!(
            "Video Track codec: {}",
            t.codec_name.clone().unwrap_or_default()
        );
        // println!("Video Track {}", t.language.unwrap().into());
    }
    println!("Title : {:?}", matroska.info.title);
    Ok(())
}

#[instrument(skip(app_state))]
async fn list_files(State(app_state): State<App>) -> Json<Option<Vec<FileInfo>>> {
    let files = app_state.indexer.files.lock().unwrap().clone();
    Json(files)

    // json!(*app_state.indexer.files.lock().unwrap());
}

async fn create_shared_link(
    State(app_state): State<App>,
    Json(files): Json<Vec<String>>,
) -> Json<Option<String>> {
    // Validate input
    for file in &files {
        if file.contains("..") || file.contains("\0") {
            return Json(None);
        }
    }

    match publish_files(files, &ServerConfig::new().host, &app_state.db_pool).await {
        Ok(link) => Json(Some(link)),
        Err(_) => Json(None),
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
            // if std::path::Path::new(&filename).extension() == Some(OsStr::new("mkv")) {
            //     get_matroska_info(&filename)?;
            // }
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
                Err(_) => return Err(anyhow::Error::msg("failed to create share link")),
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
                return Err(anyhow::Error::msg("failed to create share link"));
            }
        };
    }
    Err(anyhow::Error::msg("failed to create share link"))
}

pub struct ServerConfig {
    pub port: u16,
    pub base_path: String,
    pub host: String,
    pub data_dir: PathBuf,
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
            data_dir: Self::data_dir_from_env(),
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

/// The handler for the HTTP request (this gets called when the HTTP GET lands at the start
/// of websocket negotiation). After this completes, the actual switching from HTTP to
/// websocket protocol will occur.
/// This is the last point where we can extract TCP/IP metadata such as IP address of the client
/// as well as things from HTTP headers such as user-agent of the browser etc.
async fn ws_handler(
    State(app_state): State<App>,
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> impl IntoResponse {
    // finalize the upgrade process by returning upgrade callback.
    // we can customize the callback by sending additional info such as address.
    ws.on_upgrade(move |socket| handle_socket(socket, addr, app_state))
}

async fn handle_socket(mut socket: WebSocket, who: SocketAddr, app_state: App) {
    tracing::info!("Websocket connection from: {}", who);
    let mut rx = app_state.progress_channel_sender.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    if let Err(err) = socket
                        .send(axum::extract::ws::Message::Text(
                            serde_json::json!(msg).to_string(),
                        ))
                        .await
                    {
                        tracing::error!("WS socket send error: {}", err);
                        break;
                    }
                }
                Err(err) => {
                    tracing::error!("WS channel recv error: {}", err);
                    break;
                }
            }
        }
    });
}

#[tokio::main]
async fn main() -> Result<()> {
    pretty_env_logger::init();

    let cli = Cli::parse();
    let server_config = ServerConfig::new();
    let db_pool = init_db(server_config.data_dir).await;

    if cli.files.is_empty() && !cli.server {
        // let out = std::io::stdout();
        Cli::command().print_long_help()?;
    }

    if !cli.files.is_empty() {
        let shared_link = publish_files(cli.files, &server_config.host, &db_pool).await?;
        println!("Shared link: {}", shared_link);
    }

    if cli.server {
        init_tracing_opentelemetry::tracing_subscriber_ext::init_subscribers()?;
        let mut progress_manager = progress::Manager::new(db_pool.clone());
        // let base_path = "/mnt";
        let indexer =
            file_indexer::FileIndexer::new(&PathBuf::from(&server_config.base_path.as_str()), 60);

        let progress_channel_sender = progress_manager.sender.clone();
        progress_manager.start_recv_thread().await;

        let app_state = App::new(db_pool, progress_channel_sender, indexer);
        info!("Sarting server on port {}", server_config.port);

        let api_routes = axum::Router::new().route("/admin/list_files", get(list_shared_files));

        let app = axum::Router::new()
            .route("/s/:share_id", get(list_shared_files))
            .route("/s/:share_id/:file_id", get(download_file))
            //   .route("/admin/files", get(list_files))
            // .route("/admin/download_link", post(download_link_create))
            // .route("/admin/download_link", delete(download_link_delete))
            // .route("/admin/zip", post(zip_files))
            // .route("/admin/tasks_status", get(tasks_status))
            // .route("/admin/torrents/:id", delete(delete_torrent))
            .route("/healthcheck", get(healthcheck))
            .nest("/api", api_routes)
            .nest_service("/assets", ServeDir::new("dist/"))
            .route("/admin/live_update", get(ws_handler))
            .route("/admin/list_files", get(list_files))
            .route("/admin/create_shared_link", post(create_shared_link))
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
