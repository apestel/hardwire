use axum::{
    Json, Router,
    extract::{FromRequestParts, Path, Query, State, WebSocketUpgrade, ws::WebSocket},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
};
use serde::{Deserialize, Serialize};

use std::{fmt::Debug, sync::Arc};

use crate::{
    App,
    error::{AppError, AuthErrorKind},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    #[allow(dead_code)]
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
            .ok_or_else(|| AppError::AuthError(AuthErrorKind::MissingToken).into_response())?;

        // Get app state to access DB
        let state = parts.extensions.get::<Arc<App>>().ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("App state not found")).into_response()
        })?;

        // Get JWT secret from config
        let jwt_secret = state.config.auth.jwt_secret.as_bytes();

        // Validate JWT token
        let token_data = decode::<Claims>(
            &auth_header,
            &DecodingKey::from_secret(jwt_secret),
            &Validation::default(),
        )
        .map_err(|_| AppError::AuthError(AuthErrorKind::InvalidToken).into_response())?;

        // Get user from database
        let user = sqlx::query_as!(
            AdminUser,
            "SELECT * FROM admin_users WHERE id = ?",
            token_data.claims.sub
        )
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|e| AppError::Database(e).into_response())?
        .ok_or_else(|| AppError::AuthError(AuthErrorKind::Unauthorized).into_response())?;

        Ok(Self { user })
    }
}

#[derive(Debug, Deserialize)]
pub struct AuthCallbackQuery {
    code: String,
    state: String,
}

pub async fn google_login(State(app): State<App>) -> Result<Redirect, AppError> {
    // Build HTTP client with no redirects to prevent SSRF
    let http_client = reqwest::ClientBuilder::new()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to build HTTP client: {}", e)))?;

    let client_id = ClientId::new(app.config.auth.google_client_id.clone());
    let client_secret = ClientSecret::new(app.config.auth.google_client_secret.clone());
    let issuer_url = IssuerUrl::new("https://accounts.google.com".to_string())
        .map_err(|e| AppError::AuthError(AuthErrorKind::OAuthError(e.to_string())))?;
    let redirect_url = RedirectUrl::new(app.config.auth.google_redirect_url.clone())
        .map_err(|e| AppError::AuthError(AuthErrorKind::OAuthError(e.to_string())))?;

    // Discover Google's OIDC metadata
    let provider_metadata = CoreProviderMetadata::discover_async(issuer_url, &http_client)
        .await
        .map_err(|e| {
            AppError::AuthError(AuthErrorKind::OAuthError(format!(
                "Discovery failed: {:?}",
                e
            )))
        })?;

    // Create the OIDC client from provider metadata
    let client =
        CoreClient::from_provider_metadata(provider_metadata, client_id, Some(client_secret))
            .set_redirect_uri(redirect_url);

    // Generate PKCE challenge
    let (pkce_challenge, _pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    // Generate the authorization URL
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

    // Store PKCE verifier and nonce for later verification
    let mut pending_auths = app.pending_auths.lock().await;
    pending_auths.insert(csrf_token.secret().clone(), nonce);

    Ok(Redirect::to(auth_url.as_str()))
}

pub async fn google_callback(
    State(app): State<App>,
    Query(query): Query<AuthCallbackQuery>,
) -> Result<Json<AuthResponse>, AppError> {
    // Build HTTP client with no redirects to prevent SSRF
    let http_client = reqwest::ClientBuilder::new()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to build HTTP client: {}", e)))?;

    let client_id = ClientId::new(app.config.auth.google_client_id.clone());
    let client_secret = ClientSecret::new(app.config.auth.google_client_secret.clone());
    let issuer_url = IssuerUrl::new("https://accounts.google.com".to_string())
        .map_err(|e| AppError::AuthError(AuthErrorKind::OAuthError(e.to_string())))?;
    let redirect_url = RedirectUrl::new(app.config.auth.google_redirect_url.clone())
        .map_err(|e| AppError::AuthError(AuthErrorKind::OAuthError(e.to_string())))?;

    // Discover Google's OIDC metadata
    let provider_metadata = CoreProviderMetadata::discover_async(issuer_url, &http_client)
        .await
        .map_err(|e| {
            AppError::AuthError(AuthErrorKind::OAuthError(format!(
                "Discovery failed: {:?}",
                e
            )))
        })?;

    // Create the OIDC client from provider metadata
    let client =
        CoreClient::from_provider_metadata(provider_metadata, client_id, Some(client_secret))
            .set_redirect_uri(redirect_url);

    // Retrieve and remove the stored nonce and PKCE verifier
    let nonce = {
        let mut pending_auths = app.pending_auths.lock().await;
        pending_auths
            .remove(&query.state)
            .ok_or_else(|| AppError::AuthError(AuthErrorKind::InvalidToken))?
    };

    // Exchange the authorization code for an access token
    let token_response = client
        .exchange_code(AuthorizationCode::new(query.code)).map_err(|e| AppError::AuthError(AuthErrorKind::OAuthError(format!("Exchange code error: {:?}", e))))?
        .request_async(&http_client)
        .await
        .map_err(|e| {
            AppError::AuthError(AuthErrorKind::OAuthError(format!(
                "Token exchange failed: {:?}",
                e
            )))
        })?;

    // Extract and verify ID token
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

    // Extract user information
    let google_id = claims.subject().to_string();
    let email = claims
        .email()
        .map(|e| e.as_str().to_string())
        .ok_or_else(|| {
            AppError::AuthError(AuthErrorKind::OAuthError("No email in claims".to_string()))
        })?;

    // Check if user exists, or create new user
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
            // Create new user
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

    // Generate JWT token
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

    Ok(Json(AuthResponse { token, user }))
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ApiResponse<T> {
    Success(T),
    Error(ApiError),
}

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
}

#[derive(Debug, Serialize)]
pub struct ApiError {
    error_type: String,
    error_message: String,
}

impl<T: Serialize> IntoResponse for ApiResponse<T> {
    fn into_response(self) -> Response {
        match self {
            ApiResponse::Success(data) => (StatusCode::OK, Json(data)).into_response(),
            ApiResponse::Error(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
            }
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        ApiError {
            error_type: "InternalError".to_string(),
            error_message: err.to_string(),
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        ApiError {
            error_type: "DatabaseError".to_string(),
            error_message: err.to_string(),
        }
    }
}

pub async fn list_users(State(app): State<App>) -> Result<Json<Vec<AdminUser>>, AppError> {
    let users = sqlx::query_as!(AdminUser, "SELECT * FROM admin_users")
        .fetch_all(&app.db_pool)
        .await
        .map_err(AppError::Database)?;

    Ok(Json(users))
}

pub async fn create_user(
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
    State(app): State<App>,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    sqlx::query!("DELETE FROM admin_users WHERE id = ?", id)
        .execute(&app.db_pool)
        .await
        .map_err(AppError::Database)?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn create_task(State(_app): State<App>) -> Result<String, AppError> {
    // Placeholder implementation
    Ok("Task created".to_string())
}

pub async fn get_task_status(
    State(_app): State<App>,
    Path(_task_id): Path<String>,
) -> Result<String, AppError> {
    // Placeholder implementation
    Ok("Task status".to_string())
}

pub async fn list_files(State(_app): State<App>) -> Result<Json<Vec<String>>, AppError> {
    // Placeholder implementation
    Ok(Json(vec![]))
}

pub async fn download_stats(State(app): State<App>) -> Result<Json<DownloadStats>, AppError> {
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
        sqlx::query_scalar("SELECT COUNT(*) FROM download WHERE status = 'completed'")
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
    State(app): State<App>,
    Query(query): Query<PeriodQuery>,
) -> Result<Json<Vec<DownloadRecord>>, AppError> {
    let limit = query.limit.unwrap_or(100);

    let downloads: Vec<DownloadRecord> = sqlx::query_as(
        "SELECT id, file_path, ip_address, transaction_id, status, file_size, started_at, finished_at FROM download ORDER BY started_at DESC LIMIT ?"
    )
    .bind(limit)
    .fetch_all(&app.db_pool)
    .await
    .map_err(AppError::Database)?;

    Ok(Json(downloads))
}

#[derive(Debug, Serialize)]
pub struct StatusDistribution {
    status: String,
    count: i64,
    percentage: f64,
}

pub async fn download_status_distribution(
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

#[derive(Debug, Deserialize)]
pub struct CreateSharedLinkRequest {
    file_path: String,
    expires_at: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SharedLinkResponse {
    id: String,
    url: String,
    expires_at: Option<i64>,
}

pub async fn create_shared_link(
    State(app): State<App>,
    Json(request): Json<CreateSharedLinkRequest>,
) -> Result<Json<SharedLinkResponse>, AppError> {
    let id = nanoid::nanoid!();
    let now = chrono::Utc::now().timestamp();
    let expires_at = request.expires_at.unwrap_or(now + 86400 * 7); // 7 days default

    sqlx::query("INSERT INTO share_links (id, expiration, created_at) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(expires_at)
        .bind(now)
        .execute(&app.db_pool)
        .await
        .map_err(AppError::Database)?;

    let url = format!("{}/shared/{}", app.config.server.host, id);

    Ok(Json(SharedLinkResponse {
        id,
        url,
        expires_at: Some(expires_at),
    }))
}

async fn ws_handler(ws: WebSocketUpgrade, State(_app): State<App>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket))
}

async fn handle_socket(mut socket: WebSocket) {
    while let Some(msg) = socket.recv().await {
        if let Ok(msg) = msg {
            if socket.send(msg).await.is_err() {
                break;
            }
        }
    }
}

pub fn admin_router() -> Router<App> {
    Router::new()
        .route("/auth/google/login", get(google_login))
        .route("/auth/google/callback", get(google_callback))
        .route("/api/users", get(list_users).post(create_user))
        .route("/api/users/:id", get(get_user).delete(delete_user))
        .route("/api/tasks", post(create_task))
        .route("/api/tasks/:task_id", get(get_task_status))
        // .route("/live_update", get(ws_handler))
        .route("/api/list_files", get(list_files))
        .route("/api/create_shared_link", post(create_shared_link))
        // Nouvelles routes pour les statistiques de téléchargement
        .route("/api/stats/downloads", get(download_stats))
        .route(
            "/api/stats/downloads/by_period",
            get(download_stats_by_period),
        )
        .route("/api/stats/downloads/recent", get(recent_downloads))
        .route(
            "/api/stats/downloads/status",
            get(download_status_distribution),
        )
}
