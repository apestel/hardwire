use axum::{
    extract::{ws::WebSocket, ConnectInfo, FromRequestParts, Path, State, WebSocketUpgrade},
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use oauth2::{
    basic::BasicClient, AuthUrl, ClientId, ClientSecret, CsrfToken, RedirectUrl, Scope,
    TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::{fmt::Debug, sync::Arc};
use tokio::net::unix::SocketAddr;

use crate::App;

const JWT_SECRET: &[u8] = b"your-secret-key"; // In production, use an environment variable

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: i64, // user id
    exp: usize,
    email: String,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct AdminUser {
    pub id: i64,
    pub email: String,
    pub google_id: String,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AdminUserCreate {
    pub email: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    token: String,
    user: AdminUser,
}

pub struct AdminAuthMiddleware {
    pub user: AdminUser,
}

impl<S> FromRequestParts<S> for AdminAuthMiddleware
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Get the Authorization header
        let auth_header = parts
            .headers
            .get("Authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|auth_str| {
                if auth_str.starts_with("Bearer ") {
                    Some(auth_str[7..].to_string())
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                (StatusCode::UNAUTHORIZED, "No valid authorization header").into_response()
            })?;

        // Validate JWT token
        let token_data = decode::<Claims>(
            &auth_header,
            &DecodingKey::from_secret(JWT_SECRET),
            &Validation::default(),
        )
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid token").into_response())?;

        // Get app state to access DB
        let state = parts.extensions.get::<Arc<App>>().ok_or_else(|| {
            (StatusCode::INTERNAL_SERVER_ERROR, "App state not found").into_response()
        })?;

        // Get user from database
        let user = sqlx::query_as!(
            AdminUser,
            "SELECT * FROM admin_users WHERE id = ?",
            token_data.claims.sub
        )
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response())?
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "User not found").into_response())?;

        Ok(Self { user })
    }
}

pub fn create_oauth_client() -> BasicClient {
    let client_id = ClientId::new(
        std::env::var("GOOGLE_CLIENT_ID").expect("Missing GOOGLE_CLIENT_ID environment variable."),
    );
    let client_secret = ClientSecret::new(
        std::env::var("GOOGLE_CLIENT_SECRET")
            .expect("Missing GOOGLE_CLIENT_SECRET environment variable."),
    );
    let auth_url = AuthUrl::new("https://accounts.google.com/o/oauth2/v2/auth".to_string())
        .expect("Invalid authorization endpoint URL");
    let token_url = TokenUrl::new("https://oauth2.googleapis.com/token".to_string())
        .expect("Invalid token endpoint URL");

    BasicClient::new(client_id, Some(client_secret), auth_url, Some(token_url)).set_redirect_uri(
        RedirectUrl::new(format!(
            "{}/admin/auth/google/callback",
            std::env::var("APP_URL").expect("Missing APP_URL environment variable")
        ))
        .expect("Invalid redirect URL"),
    )
}

pub async fn init_db(_pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    // Table creation is now handled by migrations
    Ok(())
}

pub async fn google_login(State(_app): State<App>) -> impl IntoResponse {
    let client = create_oauth_client();
    let (auth_url, _csrf_token) = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("email".to_string()))
        .add_scope(Scope::new("profile".to_string()))
        .url();

    axum::response::Redirect::to(auth_url.as_str())
}

#[derive(Debug, Deserialize)]
struct AuthCallbackParams {
    code: String,
    state: String,
    // We could add other parameters if needed
}

pub async fn google_callback(
    State(app): State<App>,
    axum::extract::Query(params): axum::extract::Query<AuthCallbackParams>,
) -> Result<impl IntoResponse, Response> {
    // Exchange the code with a token
    let client = create_oauth_client();
    let token = client
        .exchange_code(oauth2::AuthorizationCode::new(params.code))
        .request_async(oauth2::reqwest::async_http_client)
        .await
        .map_err(|_| (StatusCode::BAD_REQUEST, "Failed to exchange code").into_response())?;

    // Get user info from Google
    let client = reqwest::Client::new();
    let user_info: serde_json::Value = client
        .get("https://www.googleapis.com/oauth2/v2/userinfo")
        .bearer_auth(token.access_token().secret())
        .send()
        .await
        .map_err(|_| {
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to get user info").into_response()
        })?
        .json()
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to parse user info",
            )
                .into_response()
        })?;

    let _ = user_info["email"]
        .as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "No email in response").into_response())?;
    let google_id = user_info["id"]
        .as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "No id in response").into_response())?;

    println!("Google ID: {}", google_id);
    // Check if user exists and is authorized
    let user = sqlx::query_as!(
        AdminUser,
        "SELECT * FROM admin_users WHERE google_id = ?",
        google_id
    )
    .fetch_optional(&app.db_pool)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response())?;

    let user = match user {
        Some(user) => user,
        None => {
            return Err((
                StatusCode::UNAUTHORIZED,
                "You are not authorized to access this area",
            )
                .into_response())
        }
    };

    // Create JWT token
    let expiration = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::days(7))
        .expect("valid timestamp")
        .timestamp() as usize;

    let claims = Claims {
        sub: user.id,
        exp: expiration,
        email: user.email.clone(),
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(JWT_SECRET),
    )
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Failed to create token").into_response())?;

    // Return token and user info
    Ok(Json(AuthResponse { token, user }))
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ApiResponse<T> {
    Success(T),
    Error(ApiError),
}

#[derive(Debug, Serialize)]
pub struct ApiError {
    error_type: String,
    error_message: String,
}

impl<T> IntoResponse for ApiResponse<T>
where
    T: Serialize,
{
    fn into_response(self) -> Response {
        match serde_json::to_string(&self) {
            Ok(json) => (StatusCode::OK, Json(json)).into_response(),
            Err(e) => ApiResponse::<()>::Error(ApiError {
                error_type: "serialization_error".to_string(),
                error_message: e.to_string(),
            })
            .into_response(),
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        ApiError {
            error_type: "internal_error".to_string(),
            error_message: err.to_string(),
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        ApiError {
            error_type: "database_error".to_string(),
            error_message: err.to_string(),
        }
    }
}

pub async fn list_users(State(app): State<App>, _auth: AdminAuthMiddleware) -> impl IntoResponse {
    match sqlx::query_as!(AdminUser, "SELECT * FROM admin_users")
        .fetch_all(&app.db_pool)
        .await
    {
        Ok(users) => ApiResponse::Success(users),
        Err(err) => ApiResponse::Error(err.into()),
    }
}

pub async fn create_user(
    State(app): State<App>,
    _auth: AdminAuthMiddleware,
    Json(payload): Json<AdminUserCreate>,
) -> impl IntoResponse {
    let now = chrono::Utc::now().timestamp();
    let result = sqlx::query_as!(
        AdminUser,
        r#"
        INSERT INTO admin_users (email, google_id, created_at) VALUES (?, '', ?) RETURNING *
        "#,
        payload.email,
        now
    )
    .fetch_one(&app.db_pool)
    .await;

    match result {
        Ok(user) => ApiResponse::Success(user),
        Err(err) => ApiResponse::Error(err.into()),
    }
}

pub async fn get_user(
    State(app): State<App>,
    _auth: AdminAuthMiddleware,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> impl IntoResponse {
    let result = sqlx::query_as!(AdminUser, "SELECT * FROM admin_users WHERE id = ?", id)
        .fetch_optional(&app.db_pool)
        .await;

    match result {
        Ok(Some(user)) => ApiResponse::Success(user),
        Ok(None) => ApiResponse::Error(ApiError {
            error_type: "not_found".to_string(),
            error_message: format!("User with id {} not found", id),
        }),
        Err(err) => ApiResponse::Error(err.into()),
    }
}

pub async fn delete_user(
    State(app): State<App>,
    _auth: AdminAuthMiddleware,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> impl IntoResponse {
    let result = sqlx::query("DELETE FROM admin_users WHERE id = ?")
        .bind(id)
        .execute(&app.db_pool)
        .await;

    match result {
        Ok(result) if result.rows_affected() > 0 => ApiResponse::Success(()),
        Ok(_) => ApiResponse::Error(ApiError {
            error_type: "not_found".to_string(),
            error_message: format!("User with id {} not found", id),
        }),
        Err(err) => ApiResponse::Error(err.into()),
    }
}

pub async fn create_task(
    State(app_state): State<App>,
    _auth: AdminAuthMiddleware,
    Json(input): Json<crate::worker::TaskInput>,
) -> impl IntoResponse {
    match app_state.task_manager.create_task(input).await {
        Ok(task_id) => ApiResponse::Success(task_id),
        Err(err) => ApiResponse::Error(err.into()),
    }
}

pub async fn get_task_status(
    State(app_state): State<App>,
    _auth: AdminAuthMiddleware,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    match app_state.task_manager.get_task_status(&task_id).await {
        Ok(task) => ApiResponse::Success(task),
        Err(err) => ApiResponse::Error(err.into()),
    }
}

pub async fn list_files(
    State(app_state): State<App>,
    _auth: AdminAuthMiddleware,
) -> impl IntoResponse {
    let files = app_state.indexer.files.lock().unwrap().as_ref().cloned();
    ApiResponse::Success(files)
}

pub async fn create_shared_link(
    State(app_state): State<App>,
    _auth: AdminAuthMiddleware,
    Json(files): Json<Vec<String>>,
) -> impl IntoResponse {
    // Create a vector to store file IDs and generate a unique share ID
    let mut files_id: Vec<i64> = vec![];
    let share_id = nanoid::nanoid!(10);

    // Process each file
    for filename in files {
        if std::path::Path::new(&filename).exists() {
            // Open the file
            let file_result = tokio::fs::File::open(&filename).await;
            if let Err(e) = file_result {
                return ApiResponse::Error(ApiError {
                    error_type: "internal_error".to_string(),
                    error_message: e.to_string(),
                });
            }
            let file = file_result.unwrap();

            // Get file metadata
            let metadata_result = file.metadata().await;
            if let Err(e) = metadata_result {
                return ApiResponse::Error(ApiError {
                    error_type: "internal_error".to_string(),
                    error_message: e.to_string(),
                });
            }
            let metadata = metadata_result.unwrap();
            let file_size = i64::try_from(metadata.len()).unwrap();

            // Insert file into database
            let insert_result = sqlx::query!(
                "INSERT INTO files (sha256, path, file_size) VALUES ($1, $2, $3)",
                "",
                filename,
                file_size
            )
            .execute(&app_state.db_pool)
            .await;

            match insert_result {
                Ok(row) => files_id.push(row.last_insert_rowid()),
                Err(e) => {
                    return ApiResponse::Error(ApiError::from(e));
                }
            }
        }
    }

    // If we have files, create a share link
    if !files_id.is_empty() {
        let now = chrono::offset::Utc::now().timestamp();

        // Insert share link
        let share_result = sqlx::query!(
            "INSERT INTO share_links (id, expiration, created_at) VALUES ($1, $2, $3)",
            share_id,
            -1,
            now
        )
        .execute(&app_state.db_pool)
        .await;

        match share_result {
            Ok(_) => {
                // Associate files with share link
                for id in files_id {
                    let link_result = sqlx::query!(
                        "INSERT INTO share_link_files (share_link_id, file_id) VALUES ($1, $2)",
                        share_id,
                        id
                    )
                    .execute(&app_state.db_pool)
                    .await;

                    if let Err(e) = link_result {
                        return ApiResponse::Error(ApiError {
                            error_type: "internal_error".to_string(),
                            error_message: e.to_string(),
                        });
                    }
                }

                // Return success with share link URL
                return ApiResponse::Success(Some(format!(
                    "{}/s/{}",
                    app_state.server_config.base_path, share_id
                )));
            }
            Err(e) => {
                log::error!("{}", e);
                return ApiResponse::Error(ApiError::from(e));
            }
        }
    }

    // Return error if no valid files were provided
    ApiResponse::Error(ApiError {
        error_type: "bad_request".to_string(),
        error_message: "No valid files provided".to_string(),
    })
}

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
    tracing::info!("Websocket connection from: {:#?}", who);
    let mut rx = app_state.progress_channel_sender.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    if let Err(err) = socket
                        .send(axum::extract::ws::Message::Text(
                            serde_json::json!(msg).to_string().into(),
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

pub fn admin_router() -> Router<App> {
    Router::new()
        .route("/auth/google/login", get(google_login))
        .route("/auth/google/callback", get(google_callback))
        .route("/api/users", get(list_users).post(create_user))
        .route("/api/users/{id}", get(get_user).delete(delete_user))
        .route("/api/tasks", post(create_task))
        .route("/api/tasks/{task_id}", get(get_task_status))
        // .route("/live_update", get(ws_handler))
        .route("/api/list_files", get(list_files))
        .route("/api/create_shared_link", post(create_shared_link))
}
