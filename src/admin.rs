use axum::{
    Json, Router,
    body::Body,
    extract::{FromRef, FromRequestParts, Path, Query, State, WebSocketUpgrade, ws::WebSocket},
    http::{HeaderMap, StatusCode, request::Parts},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge,
    RedirectUrl, Scope, TokenResponse,
};
use serde::{Deserialize, Serialize};
use tokio_util::codec::{BytesCodec, FramedRead};

use std::fmt::Debug;

use crate::{
    App,
    error::{AppError, AuthErrorKind},
    worker,
};

use axum::http::header::{CONTENT_DISPOSITION, CONTENT_LENGTH};

// ─── Auth types ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Claims {
    sub: i64,
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


// ─── Auth middleware ─────────────────────────────────────────────────────────

pub struct AdminAuthMiddleware {
    #[allow(dead_code)]
    pub user: AdminUser,
}

impl<S> FromRequestParts<S> for AdminAuthMiddleware
where
    App: axum::extract::FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
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
            .ok_or_else(|| AppError::AuthError(AuthErrorKind::MissingToken).into_response())?;

        let app = App::from_ref(state);
        let jwt_secret = app.config.auth.jwt_secret.clone();
        let jwt_secret = jwt_secret.as_bytes();

        let token_data = decode::<Claims>(
            &auth_header,
            &DecodingKey::from_secret(jwt_secret),
            &Validation::default(),
        )
        .map_err(|_| AppError::AuthError(AuthErrorKind::InvalidToken).into_response())?;

        let user = sqlx::query_as!(
            AdminUser,
            "SELECT * FROM admin_users WHERE id = ?",
            token_data.claims.sub
        )
        .fetch_optional(&app.db_pool)
        .await
        .map_err(|e| AppError::Database(e).into_response())?
        .ok_or_else(|| AppError::AuthError(AuthErrorKind::Unauthorized).into_response())?;

        Ok(Self { user })
    }
}

// ─── OAuth ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AuthCallbackQuery {
    code: String,
    state: String,
}

/// Returns cached OIDC provider metadata. Discovery is performed once and reused.
async fn oidc_provider_metadata(app: &App) -> Result<CoreProviderMetadata, AppError> {
    app.oidc_metadata
        .get_or_try_init(|| async {
            let http_client = reqwest::ClientBuilder::new()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| {
                    AppError::Internal(anyhow::anyhow!("Failed to build HTTP client: {}", e))
                })?;
            let issuer_url = IssuerUrl::new("https://accounts.google.com".to_string())
                .map_err(|e| AppError::AuthError(AuthErrorKind::OAuthError(e.to_string())))?;
            CoreProviderMetadata::discover_async(issuer_url, &http_client)
                .await
                .map_err(|e| {
                    AppError::AuthError(AuthErrorKind::OAuthError(format!(
                        "Discovery failed: {:?}",
                        e
                    )))
                })
        })
        .await
        .cloned()
}

pub async fn google_login(State(app): State<App>) -> Result<Redirect, AppError> {
    let metadata = oidc_provider_metadata(&app).await?;
    let client_id = ClientId::new(app.config.auth.google_client_id.clone());
    let client_secret = ClientSecret::new(app.config.auth.google_client_secret.clone());
    let redirect_url = RedirectUrl::new(app.config.auth.google_redirect_url.clone())
        .map_err(|e| AppError::AuthError(AuthErrorKind::OAuthError(e.to_string())))?;
    let client = CoreClient::from_provider_metadata(metadata, client_id, Some(client_secret))
        .set_redirect_uri(redirect_url);

    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    let (auth_url, csrf_token, nonce) = client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .add_scope(Scope::new("openid".to_string()))
        .add_scope(Scope::new("email".to_string()))
        .add_scope(Scope::new("profile".to_string()))
        .set_pkce_challenge(pkce_challenge)
        .url();

    let now = chrono::Utc::now().timestamp();
    let mut pending_auths = app.pending_auths.lock().await;

    // Purge entries older than 10 minutes to prevent unbounded growth
    pending_auths.retain(|_, (_, _, inserted_at)| now - *inserted_at < 600);

    pending_auths.insert(csrf_token.secret().clone(), (nonce, pkce_verifier, now));

    Ok(Redirect::to(auth_url.as_str()))
}

pub async fn google_callback(
    State(app): State<App>,
    Query(query): Query<AuthCallbackQuery>,
) -> Result<Redirect, AppError> {
    let http_client = reqwest::ClientBuilder::new()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to build HTTP client: {}", e)))?;

    let metadata = oidc_provider_metadata(&app).await?;
    let client_id = ClientId::new(app.config.auth.google_client_id.clone());
    let client_secret = ClientSecret::new(app.config.auth.google_client_secret.clone());
    let redirect_url = RedirectUrl::new(app.config.auth.google_redirect_url.clone())
        .map_err(|e| AppError::AuthError(AuthErrorKind::OAuthError(e.to_string())))?;
    let client = CoreClient::from_provider_metadata(metadata, client_id, Some(client_secret))
        .set_redirect_uri(redirect_url);

    let (nonce, pkce_verifier, _) = {
        let mut pending_auths = app.pending_auths.lock().await;
        pending_auths
            .remove(&query.state)
            .ok_or_else(|| AppError::AuthError(AuthErrorKind::InvalidToken))?
    };

    let token_response = client
        .exchange_code(AuthorizationCode::new(query.code))
        .map_err(|e| {
            AppError::AuthError(AuthErrorKind::OAuthError(format!(
                "Exchange code error: {:?}",
                e
            )))
        })?
        .set_pkce_verifier(pkce_verifier)
        .request_async(&http_client)
        .await
        .map_err(|e| {
            AppError::AuthError(AuthErrorKind::OAuthError(format!(
                "Token exchange failed: {:?}",
                e
            )))
        })?;

    let id_token = token_response
        .id_token()
        .ok_or_else(|| AppError::AuthError(AuthErrorKind::OAuthError("No ID token".to_string())))?;

    let id_token_verifier = client.id_token_verifier();
    let claims = id_token.claims(&id_token_verifier, &nonce).map_err(|e| {
        AppError::AuthError(AuthErrorKind::OAuthError(format!(
            "ID token verification failed: {:?}",
            e
        )))
    })?;

    let google_id = claims.subject().to_string();
    let email = claims
        .email()
        .map(|e| e.as_str().to_string())
        .ok_or_else(|| {
            AppError::AuthError(AuthErrorKind::OAuthError("No email in claims".to_string()))
        })?;

    let user = sqlx::query_as!(
        AdminUser,
        "SELECT * FROM admin_users WHERE google_id = ?",
        google_id
    )
    .fetch_optional(&app.db_pool)
    .await
    .map_err(AppError::Database)?;

    let user = match user {
        Some(u) => u,
        None => {
            let now = chrono::Utc::now().timestamp();
            sqlx::query!(
                "INSERT INTO admin_users (email, google_id, created_at) VALUES (?, ?, ?)",
                email,
                google_id,
                now
            )
            .execute(&app.db_pool)
            .await
            .map_err(AppError::Database)?;

            sqlx::query_as!(
                AdminUser,
                "SELECT * FROM admin_users WHERE google_id = ?",
                google_id
            )
            .fetch_one(&app.db_pool)
            .await
            .map_err(AppError::Database)?
        }
    };

    let jwt_secret = app.config.auth.jwt_secret.as_bytes();
    let exp = (chrono::Utc::now() + chrono::Duration::days(7)).timestamp() as usize;
    let jwt_claims = Claims {
        sub: user.id,
        exp,
        email: user.email.clone(),
    };

    let token = encode(
        &Header::default(),
        &jwt_claims,
        &EncodingKey::from_secret(jwt_secret),
    )
    .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to generate token: {}", e)))?;

    // JWT uses base64url encoding — no special URL encoding needed
    Ok(Redirect::to(&format!("/admin/auth/done?token={}", token)))
}

// ─── User management ─────────────────────────────────────────────────────────

pub async fn list_users(
    _auth: AdminAuthMiddleware,
    State(app): State<App>,
) -> Result<Json<Vec<AdminUser>>, AppError> {
    let users = sqlx::query_as!(AdminUser, "SELECT * FROM admin_users")
        .fetch_all(&app.db_pool)
        .await
        .map_err(AppError::Database)?;
    Ok(Json(users))
}

pub async fn create_user(
    _auth: AdminAuthMiddleware,
    State(app): State<App>,
    Json(user_create): Json<AdminUserCreate>,
) -> Result<Json<AdminUser>, AppError> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query!(
        "INSERT INTO admin_users (email, google_id, created_at) VALUES (?, ?, ?)",
        user_create.email,
        "",
        now
    )
    .execute(&app.db_pool)
    .await
    .map_err(AppError::Database)?;

    let user = sqlx::query_as!(
        AdminUser,
        "SELECT * FROM admin_users WHERE email = ?",
        user_create.email
    )
    .fetch_one(&app.db_pool)
    .await
    .map_err(AppError::Database)?;

    Ok(Json(user))
}

pub async fn get_user(
    _auth: AdminAuthMiddleware,
    State(app): State<App>,
    Path(id): Path<i64>,
) -> Result<Json<AdminUser>, AppError> {
    let user = sqlx::query_as!(AdminUser, "SELECT * FROM admin_users WHERE id = ?", id)
        .fetch_optional(&app.db_pool)
        .await
        .map_err(AppError::Database)?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("User not found")))?;
    Ok(Json(user))
}

pub async fn delete_user(
    _auth: AdminAuthMiddleware,
    State(app): State<App>,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    sqlx::query!("DELETE FROM admin_users WHERE id = ?", id)
        .execute(&app.db_pool)
        .await
        .map_err(AppError::Database)?;
    Ok(StatusCode::NO_CONTENT)
}

// ─── Tasks ───────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct CreateTaskResponse {
    task_id: String,
}

pub async fn create_task(
    _auth: AdminAuthMiddleware,
    State(app): State<App>,
    Json(input): Json<worker::TaskInput>,
) -> Result<Json<CreateTaskResponse>, AppError> {
    let task_id = app
        .task_manager
        .create_task(input)
        .await
        .map_err(|e| AppError::TaskError(e.to_string()))?;
    Ok(Json(CreateTaskResponse { task_id }))
}

pub async fn get_task_status(
    _auth: AdminAuthMiddleware,
    State(app): State<App>,
    Path(task_id): Path<String>,
) -> Result<Json<worker::Task>, AppError> {
    let task = app
        .task_manager
        .get_task_status(&task_id)
        .await
        .map_err(|e| AppError::TaskError(e.to_string()))?;
    Ok(Json(task))
}

pub async fn download_task_output(
    _auth: AdminAuthMiddleware,
    State(app): State<App>,
    Path(task_id): Path<String>,
) -> Result<Response, AppError> {
    let row = sqlx::query!(
        "SELECT output_data, status FROM tasks WHERE id = ?",
        task_id
    )
    .fetch_optional(&app.db_pool)
    .await
    .map_err(AppError::Database)?
    .ok_or_else(|| AppError::TaskError("Task not found".to_string()))?;

    if row.status != "completed" {
        return Err(AppError::TaskError("Task not yet completed".to_string()));
    }

    let output: serde_json::Value =
        serde_json::from_str(row.output_data.as_deref().unwrap_or("{}"))
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;

    let archive_path = output["archive_path"]
        .as_str()
        .ok_or_else(|| AppError::TaskError("No archive_path in output".to_string()))?
        .to_owned();

    let file = tokio::fs::File::open(&archive_path)
        .await
        .map_err(AppError::FileSystem)?;
    let file_size = file
        .metadata()
        .await
        .map_err(AppError::FileSystem)?
        .len();

    let filename = std::path::Path::new(&archive_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("archive.7z")
        .to_owned();

    let stream = FramedRead::new(file, BytesCodec::new());
    let body = Body::from_stream(stream);

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_LENGTH, file_size.to_string().parse().unwrap());
    headers.insert(
        CONTENT_DISPOSITION,
        format!("attachment; filename=\"{}\"", filename)
            .parse()
            .unwrap(),
    );

    Ok((headers, body).into_response())
}

// ─── File listing ────────────────────────────────────────────────────────────

pub async fn list_files(
    _auth: AdminAuthMiddleware,
    State(app): State<App>,
) -> Result<Json<Vec<crate::file_indexer::FileInfo>>, AppError> {
    let files = app
        .indexer
        .files
        .lock()
        .map_err(|_| AppError::Internal(anyhow::anyhow!("FileIndexer lock poisoned")))?;
    Ok(Json(files.clone().unwrap_or_default()))
}

// ─── Share links ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateSharedLinkRequest {
    file_paths: Vec<String>,
    expires_at: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SharedLinkResponse {
    id: String,
    url: String,
    expires_at: Option<i64>,
}

pub async fn create_shared_link(
    _auth: AdminAuthMiddleware,
    State(app): State<App>,
    Json(request): Json<CreateSharedLinkRequest>,
) -> Result<Json<SharedLinkResponse>, AppError> {
    if request.file_paths.is_empty() {
        return Err(AppError::ValidationError("file_paths must not be empty".into()));
    }

    let now = chrono::Utc::now().timestamp();
    let expires_at = request.expires_at.unwrap_or(now + 86400 * 7);
    let share_id = nanoid::nanoid!(10);

    let mut tx = app.db_pool.begin().await.map_err(AppError::Database)?;

    sqlx::query!(
        "INSERT INTO share_links (id, expiration, created_at) VALUES (?, ?, ?)",
        share_id,
        expires_at,
        now
    )
    .execute(&mut *tx)
    .await
    .map_err(AppError::Database)?;

    for file_path in &request.file_paths {
        // file_path from the indexer is relative to data_dir — resolve to absolute
        let abs_path = app.config.server.data_dir.join(file_path);
        let path = abs_path.to_string_lossy().to_string();

        let metadata = tokio::fs::metadata(&abs_path)
            .await
            .map_err(|_| AppError::FileNotFound(path.clone()))?;
        let file_size = metadata.len() as i64;

        let file_id = sqlx::query!(
            "INSERT INTO files (sha256, path, file_size) VALUES (?, ?, ?)",
            "",
            path,
            file_size
        )
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?
        .last_insert_rowid();

        sqlx::query!(
            "INSERT INTO share_link_files (share_link_id, file_id) VALUES (?, ?)",
            share_id,
            file_id
        )
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;
    }

    tx.commit().await.map_err(AppError::Database)?;

    let url = format!("{}/s/{}", app.config.server.host, share_id);

    Ok(Json(SharedLinkResponse {
        id: share_id,
        url,
        expires_at: Some(expires_at),
    }))
}

// ─── Statistics ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct DownloadStats {
    pub total_downloads: i64,
    pub total_size: i64,
    pub completed_downloads: i64,
    pub average_download_time: Option<f64>,
    pub success_rate: f64,
}

#[derive(Debug, Serialize)]
pub struct DownloadsByPeriod {
    pub period: String,
    pub data: Vec<PeriodData>,
}

#[derive(Debug, Serialize)]
pub struct PeriodData {
    pub date: String,
    pub count: i64,
    pub size: i64,
}

#[derive(Debug, Deserialize)]
pub struct PeriodQuery {
    pub period: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct StatusDistribution {
    status: String,
    count: i64,
    percentage: f64,
}

pub async fn download_stats(
    _auth: AdminAuthMiddleware,
    State(app): State<App>,
) -> Result<Json<DownloadStats>, AppError> {
    let total_downloads: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM download")
        .fetch_one(&app.db_pool)
        .await
        .map_err(AppError::Database)?;

    let total_size: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(file_size), 0) FROM download WHERE file_size IS NOT NULL",
    )
    .fetch_one(&app.db_pool)
    .await
    .map_err(AppError::Database)?;

    let completed_downloads: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM download WHERE status = 'complete'")
            .fetch_one(&app.db_pool)
            .await
            .map_err(AppError::Database)?;

    let average_download_time: Option<f64> = sqlx::query_scalar(
        "SELECT AVG(finished_at - started_at) FROM download WHERE finished_at IS NOT NULL",
    )
    .fetch_one(&app.db_pool)
    .await
    .map_err(AppError::Database)?;

    let success_rate = if total_downloads > 0 {
        (completed_downloads as f64 / total_downloads as f64) * 100.0
    } else {
        0.0
    };

    Ok(Json(DownloadStats {
        total_downloads,
        total_size,
        completed_downloads,
        average_download_time,
        success_rate,
    }))
}

pub async fn download_stats_by_period(
    _auth: AdminAuthMiddleware,
    State(app): State<App>,
    Query(query): Query<PeriodQuery>,
) -> Result<Json<DownloadsByPeriod>, AppError> {
    let period = query.period.as_deref().unwrap_or("day");
    let limit = query.limit.unwrap_or(30);

    let date_format = match period {
        "hour" => "%Y-%m-%d %H:00:00",
        "day" => "%Y-%m-%d",
        "week" => "%Y-W%W",
        "month" => "%Y-%m",
        _ => "%Y-%m-%d",
    };

    let records: Vec<(String, i64, i64)> = sqlx::query_as(&format!(
        "SELECT strftime('{}', datetime(started_at, 'unixepoch')) as date,
                COUNT(*) as count,
                COALESCE(SUM(file_size), 0) as size
         FROM download
         GROUP BY date
         ORDER BY date DESC
         LIMIT ?",
        date_format
    ))
    .bind(limit)
    .fetch_all(&app.db_pool)
    .await
    .map_err(AppError::Database)?;

    let data = records
        .into_iter()
        .map(|(date, count, size)| PeriodData { date, count, size })
        .collect();

    Ok(Json(DownloadsByPeriod {
        period: period.to_string(),
        data,
    }))
}

pub async fn recent_downloads(
    _auth: AdminAuthMiddleware,
    State(app): State<App>,
    Query(query): Query<PeriodQuery>,
) -> Result<Json<Vec<DownloadRecord>>, AppError> {
    let limit = query.limit.unwrap_or(100);
    let offset = query.offset.unwrap_or(0);

    let downloads: Vec<DownloadRecord> = sqlx::query_as(
        "SELECT id, file_path, ip_address, transaction_id, status, file_size, started_at, finished_at FROM download ORDER BY finished_at DESC NULLS LAST, started_at DESC LIMIT ? OFFSET ?"
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(&app.db_pool)
    .await
    .map_err(AppError::Database)?;

    Ok(Json(downloads))
}

pub async fn download_status_distribution(
    _auth: AdminAuthMiddleware,
    State(app): State<App>,
) -> Result<Json<Vec<StatusDistribution>>, AppError> {
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM download")
        .fetch_one(&app.db_pool)
        .await
        .map_err(AppError::Database)?;

    let statuses: Vec<(String, i64)> = sqlx::query_as(
        "SELECT status, COUNT(*) as count FROM download GROUP BY status ORDER BY count DESC",
    )
    .fetch_all(&app.db_pool)
    .await
    .map_err(AppError::Database)?;

    let distribution = statuses
        .into_iter()
        .map(|(status, count)| StatusDistribution {
            status,
            count,
            percentage: if total > 0 {
                (count as f64 / total as f64) * 100.0
            } else {
                0.0
            },
        })
        .collect();

    Ok(Json(distribution))
}

// ─── Download records ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct DownloadRecord {
    pub id: i64,
    pub file_path: String,
    pub ip_address: String,
    pub transaction_id: String,
    pub status: String,
    pub file_size: Option<i64>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

// ─── WebSocket live updates ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct WsQuery {
    token: Option<String>,
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(app): State<App>,
    Query(query): Query<WsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let token = query
        .token
        .ok_or_else(|| AppError::AuthError(AuthErrorKind::MissingToken))?;

    let jwt_secret = app.config.auth.jwt_secret.as_bytes();
    decode::<Claims>(
        &token,
        &DecodingKey::from_secret(jwt_secret),
        &Validation::default(),
    )
    .map_err(|_| AppError::AuthError(AuthErrorKind::InvalidToken))?;

    Ok(ws.on_upgrade(move |socket| handle_socket(socket, app)))
}

async fn handle_socket(mut socket: WebSocket, app: App) {
    use axum::extract::ws::Message;
    use tokio::sync::broadcast::error::RecvError;

    let mut rx = app.progress_channel_sender.subscribe();

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        if let Ok(json) = serde_json::to_string(&event) {
                            if socket.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(RecvError::Closed) => break,
                    Err(RecvError::Lagged(_)) => continue,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}

// ─── File rescan ─────────────────────────────────────────────────────────────

pub async fn rescan_files(
    _auth: AdminAuthMiddleware,
    State(app): State<App>,
) -> Result<StatusCode, AppError> {
    let done_rx = app
        .indexer
        .rescan_and_wait()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("Failed to trigger rescan")))?;
    // Wait for the indexer thread to finish scanning before returning,
    // so the caller can immediately fetch fresh file listings.
    let _ = done_rx.await;
    Ok(StatusCode::NO_CONTENT)
}

// ─── Router ──────────────────────────────────────────────────────────────────

pub fn admin_router() -> Router<App> {
    Router::new()
        .route("/auth/google/login", get(google_login))
        .route("/auth/google/callback", get(google_callback))
        .route("/live_update", get(ws_handler))
        .route("/api/users", get(list_users).post(create_user))
        .route("/api/users/{id}", get(get_user).delete(delete_user))
        .route("/api/tasks", post(create_task))
        .route("/api/tasks/{task_id}", get(get_task_status))
        .route("/api/tasks/{task_id}/download", get(download_task_output))
        .route("/api/list_files", get(list_files))
        .route("/api/files/rescan", post(rescan_files))
        .route("/api/create_shared_link", post(create_shared_link))
        .route("/api/stats/downloads", get(download_stats))
        .route("/api/stats/downloads/by_period", get(download_stats_by_period))
        .route("/api/stats/downloads/recent", get(recent_downloads))
        .route("/api/stats/downloads/status", get(download_status_distribution))
        .fallback_service(
            tower_http::services::ServeDir::new("dist/admin")
                .fallback(tower_http::services::ServeFile::new("dist/admin/index.html")),
        )
}
