use axum::response::{Html, IntoResponse};
use axum::{
    body::StreamBody,
    http::{header, StatusCode},
};

use tokio_util::io::ReaderStream;

use clap::{IntoApp, Parser};
use log::{debug, info, warn};
use nanoid::nanoid;
use pretty_env_logger;
use sha2::digest::generic_array::typenum::Len;
use sqlx::{Executor, SqlitePool};
use tokio::signal;
use tokio::signal::unix::signal;

use std::convert::TryFrom;
use std::fs::File;
use std::io::{self, Seek};
use std::net::{Ipv4Addr, SocketAddr};
//use std::path::Path;
use std::{env, ffi::OsStr};

use derive_more::{Display, Error};

use matroska::Matroska;

use askama::Template;

//use hex_literal::hex;
use sha2::{Digest, Sha256};

extern crate chrono;

type Db = sqlx::SqlitePool;

use axum::extract::{Path, State};
use axum::routing::{get, post};
// use axum_extra::routing::{
//     RouterExt, // for `Router::typed_get`
//     TypedPath,
// };

#[derive(Parser, Debug)]
#[clap(about, version, author)]
struct Cli {
    /// Server
    #[clap(short, long)]
    server: bool,

    /// Filename to publish
    #[clap(short, long)]
    filename: Option<String>,
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
    if s.len() > 0 {
        return s[s.len() - 1].to_string();
    }
    s[0].to_string()
}

async fn list_shared_files(
    State(db_pool): State<SqlitePool>,
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
    .fetch_all(&db_pool)
    .await
    {
        Ok(mut rows) => {
            let server = ServerConfig::new();
            if rows.len() > 0 {
                for mut r in rows.iter_mut() {
                    r.short_filename = short_filename(r.filename.clone());
                }
                let first_filename: String = rows.iter().next().unwrap().short_filename.clone();

                let t = DownloadFilesTemplate {
                    files: rows,
                    share_id: share_id.to_string(),
                    hardwire_host: server.host,
                    first_filename: first_filename,
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

async fn download_file(
    State(db_pool): State<SqlitePool>,
    Path((share_id, file_id)): Path<(String, u32)>,
) -> impl IntoResponse {
    let file_path = match sqlx::query!(
        r#"SELECT path as file_path 
    FROM files JOIN share_link_files ON share_link_files.file_id=files.id 
    WHERE files.id=$1 AND share_link_files.share_link_id=$2"#,
        file_id,
        share_id
    )
    .fetch_one(&db_pool)
    .await
    {
        Ok(row) => row.file_path,
        Err(_) => return Err(not_found().await),
    };

    let file = match tokio::fs::File::open(file_path).await {
        Ok(file) => file,
        Err(_) => return Err(not_found().await),
    };
    let stream = ReaderStream::new(file);
    let body = StreamBody::new(stream);
    Ok(body)
}

fn get_matroska_info(filename: &String) -> std::io::Result<()> {
    let file = File::open(&filename)?;
    let matroska = Matroska::open(file).unwrap();
    let mut i = 0;
    for t in matroska.video_tracks() {
        i = i + 1;
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
        let row = match sqlx::query!(
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
    if files_id.len() > 0 {
        let share_id = nanoid::nanoid!(10);
        println!("Share ID: {}", &share_id);
        let now = chrono::offset::Utc::now().timestamp();
        let row = match sqlx::query!(
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

#[tokio::main]
async fn main() -> std::io::Result<()> {
    pretty_env_logger::init();

    let cli = Cli::parse();
    let server_config = ServerConfig::new();
    let db_pool = init_db(server_config.data_dir).await;

    if cli.filename.is_none() && !cli.server {
        let mut out = std::io::stdout();
        Cli::into_app()
            .write_long_help(&mut out)
            .expect("failed to write to stdout");
    }

    if cli.filename.is_some() {
        match publish_file(cli.filename.unwrap(), server_config.host, &db_pool).await {
            Ok(_) => println!("Job done!"),
            Err(e) => panic!("Hardwire could not proceed: {}", e),
        }
    }

    if cli.server {
        info!("Sarting server on port {}", server_config.port);
        //axum::Router::new().nest()

        let app: _ = axum::Router::with_state(db_pool)
            .route("/s/:share_id", get(list_shared_files))
            .route("/s/:share_id/:file_id", get(download_file))
            .route("/healthcheck", get(healthcheck))
            .merge(axum_extra::routing::SpaRouter::new("/assets", "dist/"));

        //  let app = app.fallback(not_found);

        let addr = SocketAddr::new(
            std::net::IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
            server_config.port,
        );
        axum::Server::bind(&addr)
            .serve(app.into_make_service())
            //  .with_graceful_shutdown(shutdown_signal)
            .await;

        //     App::new()
        //         .wrap(Logger::default())
        //         .app_data(web::Data::new(db_pool.clone()))
        //         .service(Files::new("/css", "dist/"))
        //         .service(Files::new("/images", "static/images/"))
        //         .route("/s/{share_id}", web::get().to(list_shared_files))
        //         .route("/s/{share_id}/{file_id}", web::get().to(download_file))
        //         .route("/healthcheck", web::get().to(healthcheck))
        //         .default_service(web::route().to(not_found))
        // })
        // .bind(("0.0.0.0", server_config.port))?
        // .run()
        // .await;
    }
    Ok(())
}
