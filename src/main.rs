use axum::extract::ws::WebSocket;
use axum::http::header::CONTENT_LENGTH;
use axum::response::{Html, IntoResponse};
use axum::Json;
use axum::{body::StreamBody, http::StatusCode};

use axum_tracing_opentelemetry::middleware::{OtelAxumLayer, OtelInResponseLayer};
// use qbittorrent::data::TorrentInfo;
// use qbittorrent::{data::Torrent, traits::TorrentData, Api};
use tokio::sync::broadcast;
use tokio_util::codec::{BytesCodec, FramedRead};
use tower_http::services::ServeDir;
use tracing::instrument;
use walkdir::{DirEntry, WalkDir};

use clap::{CommandFactory, Parser};
use log::{debug, error, info, warn};

use sqlx::{Pool, Sqlite, SqlitePool};

use std::collections::BTreeMap;
use std::convert::TryFrom;

use std::fs::File;

use std::net::{Ipv4Addr, SocketAddr};
use std::ops::Add;
use std::thread;
use std::{env, ffi::OsStr};

use matroska::Matroska;

use askama::Template;

extern crate chrono;

type Db = sqlx::SqlitePool;

use axum::extract::{ConnectInfo, Path, State, WebSocketUpgrade};
use axum::routing::{delete, get, post};

mod file_indexer;
mod progress;
use progress::ProgressReader;
use tracing_opentelemetry_instrumentation_sdk::find_current_trace_id;

use crate::progress::{Event, FileDownload};

#[derive(clap::Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Server
    #[arg(short, long)]
    server: bool,

    /// Filename to publish
    #[arg(short, long)]
    filename: Option<String>,
}

/// App holds the state of the application
#[derive(Clone, Debug)]
struct App {
    //file_list: Vec<(String, Option<Torrent>)>,
    db_pool: Pool<Sqlite>,
    progress_channel_sender: broadcast::Sender<progress::Event>,
}

impl App {
    fn new(
        pool: Pool<Sqlite>,
        progress_channel_sender: broadcast::Sender<progress::Event>,
    ) -> Self {
        App {
            //  file_list: Vec::new(),
            db_pool: pool,
            progress_channel_sender,
        }
    }
}

struct DownloadLink {
    id: String,
    filename: String,
    file_sha256: String,
    expiration: i64,
    created_at: i64,
}

struct Downloads {
    file_sha256: String,
    download_count: u32,
    src_ip_address: String,
}

struct TorrentInfoSimple {
    name: String,
    size: u64,
}

fn is_not_hidden(entry: &DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|s| entry.depth() == 0 || !s.starts_with("."))
        .unwrap_or(false)
}

impl App {
    // async fn update_torrent_list(&mut self) {
    //     let api = Api::new("login", "mypassword", "https://torrent.url.me:443")
    //         .await
    //         .unwrap();
    //     info!("[qbitorrent] Downloading Torrent list...");
    //     let torrents = api.get_torrent_list().await.unwrap();
    //     let mut map_filename_torrents: BTreeMap<String, Option<Torrent>> = BTreeMap::new();
    //     //let mut app = App::default();
    //     //let mut map_from_torrents: HashMap<String, Option<&Torrent>> = HashMap::new();
    //     let mut i = 0;

    //     for t in torrents {
    //         if let Ok(c) = t.contents(&api).await {
    //             for e in c {
    //                 map_filename_torrents.insert(e.name().clone(), Some(t.clone()));
    //             }
    //         }
    //         i = i.add(1);
    //         //        dbg!(t.name());
    //         //        dbg!(t.tracker());
    //         //        let torrent_info = t.contents(&api).await.unwrap();

    //         //       println!("Torrent ration : {}", t.ratio());
    //         //       println!("Torrent state: #{:?}", t.state());
    //     }
    //     //dbg!(torrents);

    //     WalkDir::new(".")
    //         .into_iter()
    //         .filter_entry(|e| is_not_hidden(e))
    //         .filter_map(|v| v.ok())
    //         .for_each(|x| {
    //             let p = x.path().display().to_string();
    //             if p.len() > 2 {
    //                 // suppress the ./ from the beginning Path
    //                 let p = &p[2..p.len()];
    //                 match map_filename_torrents.get(p) {
    //                     Some(_) => (),
    //                     None => {
    //                         map_filename_torrents.insert(p.to_string(), None);
    //                     }
    //                 }
    //             }
    //         });

    //     self.file_list = map_filename_torrents
    //         .iter()
    //         .map(|(x, y)| (x.clone(), y.clone()))
    //         .collect();
    //     info!("[qbitorrent] Torrent list downloaded [OK]");
    // }
}

async fn init_db(data_dir: String) -> Db {
    let mut p = data_dir.clone();
    let file_name = "/db.sqlite".to_string();
    p.push_str(&file_name);
    println!("SQLite Path: {}", p);

    let path = std::path::Path::new(&p);
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
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
                for mut r in rows.iter_mut() {
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
        Err(e) => (StatusCode::BAD_REQUEST, Html("".to_string())),
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
    let stream = FramedRead::new(progress_reader, BytesCodec::new());
    let body = StreamBody::new(stream);

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(CONTENT_LENGTH, file_size.to_string().parse().unwrap());

    Ok((headers, body))
}

fn get_matroska_info(filename: &String) -> std::io::Result<()> {
    let file = File::open(&filename)?;
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

async fn publish_file(
    filename: String,
    base_url: String,
    db_pool: &SqlitePool,
) -> std::io::Result<()> {
    let mut files_id: Vec<i64> = vec![];
    if std::path::Path::new(&filename).exists() {
        let file = File::open(&filename)?;
        // let mut sha256 = Sha256::new();
        // println!("Compute SHA256 for file: {}", &filename);
        // io::copy(&mut file, &mut sha256)?;
        // let hash = sha256.finalize();
        // println!("File: {} sha256: 0x{:x}", &filename, hash);
        // file.rewind()?;
        if std::path::Path::new(&filename).extension() == Some(OsStr::new("mkv")) {
            get_matroska_info(&filename)?;
        }
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
            Err(e) => println!("Could not insert {} in DB: {}", &filename, e),
        };
    }
    if !files_id.is_empty() {
        let share_id = nanoid::nanoid!(10);
        println!("Share ID: {}", &share_id);
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
                    match sqlx::query!(
                        "INSERT INTO share_link_files (share_link_id, file_id) VALUES ($1, $2)",
                        share_id,
                        id
                    )
                    .execute(db_pool)
                    .await
                    {
                        Ok(_) => {}
                        Err(e) => println!("Could not insert files in DB: {}", e),
                    }
                }
                println!("Share link: {}/s/{}", base_url, share_id);
            }
            Err(e) => println!("Could not insert share in DB {}", e),
        };
    }

    Ok(())
}

pub struct ServerConfig {
    pub port: u16,
    pub base_path: String,
    pub host: String,
    pub data_dir: String,
}

impl ServerConfig {
    const STD_PORT: u16 = 8080;
    const STD_BASE_PATH: &'static str = "/share";
    const STD_HOST: &'static str = "http://localhost:8080";
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
        env::var(ServerConfig::BASE_PATH_ENV_VAR)
            .map(|val| val)
            .unwrap_or(ServerConfig::STD_BASE_PATH.to_string())
    }

    fn host_from_env() -> String {
        env::var(ServerConfig::HOST_ENV_VAR)
            .map(|val| val)
            .unwrap_or(ServerConfig::STD_HOST.to_string())
    }

    fn data_dir_from_env() -> String {
        env::var(ServerConfig::HARDWIRE_DATA_DIR_ENV_VAR)
            .map(|val| val)
            .unwrap_or(ServerConfig::STD_HARDWIRE_DATA_DIR.to_string())
    }
}

async fn not_found() -> (StatusCode, Html<String>) {
    let t = T404 {};
    (StatusCode::NOT_FOUND, Html(t.render().unwrap()))
}

fn merge_files_with_torrent_infos() {}

// async fn list_files(State(app_state): State<App>) -> impl IntoResponse {
//     Json(app_state.file_list.clone())
// }

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
async fn main() -> std::io::Result<()> {
    pretty_env_logger::init();

    let cli = Cli::parse();
    let server_config = ServerConfig::new();
    let db_pool = init_db(server_config.data_dir).await;

    if cli.filename.is_none() && !cli.server {
        let mut out = std::io::stdout();
        Cli::command().print_long_help();
    }

    if cli.filename.is_some() {
        match publish_file(cli.filename.unwrap(), server_config.host, &db_pool).await {
            Ok(_) => println!("Job done!"),
            Err(e) => panic!("Hardwire could not proceed: {}", e),
        }
    }

    if cli.server {
        init_tracing_opentelemetry::tracing_subscriber_ext::init_subscribers().unwrap();
        let mut progress_manager = progress::Manager::new(db_pool.clone());
        // let base_path = "/mnt";
        // file_indexer::Indexer::new(base_path.to_string(), db_pool.clone()).index();
        let progress_channel_sender = progress_manager.sender.clone();
        progress_manager.start_recv_thread().await;

        let app_state = App::new(db_pool, progress_channel_sender);
        //app_state.update_torrent_list().await;
        info!("Sarting server on port {}", server_config.port);
        // let api_routes: _ = axum::Router::new()
        //     .route("/admin/files", get(list_files))
        //     .with_state(app_state);
        //axum::Router::new().nest()

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
            .with_state(app_state)
            // include trace context as header into the response
            .layer(OtelInResponseLayer)
            //start OpenTelemetry trace on incoming request
            .layer(OtelAxumLayer::default());

        //  let app = app.fallback(not_found);

        let addr = SocketAddr::new(
            std::net::IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
            server_config.port,
        );
        axum::Server::bind(&addr)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
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
