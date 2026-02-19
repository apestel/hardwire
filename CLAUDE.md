# Hardwire — CLAUDE.md

Self-hosted file sharing service (WetTransfer-like), written in Rust. Users upload files, admins create share links; recipients download via a public page.

## Stack

| Layer | Technology |
|---|---|
| Web framework | Axum 0.8 + Tower |
| Async runtime | Tokio (full) |
| Database | SQLite via sqlx 0.8 (compile-time macros, built-in migrations) |
| Templating | Askama 0.14 (compile-time Jinja2-style, HTML) |
| Auth | Google OIDC (openidconnect 4, PKCE flow) + JWT (jsonwebtoken 10, HS256) |
| Observability | OpenTelemetry (OTLP), tracing, tokio-console |
| Archiving | sevenz-rust (LZMA2 + AES-256) |
| CSS | Tailwind CSS (compiled via `npx @tailwindcss/cli`) |
| CLI | clap 4 (derive) |

## Project Structure

```
src/
  main.rs          # Entry point, App state, router, download handlers, templates
  lib.rs           # Re-exports config and error modules for integration tests
  admin.rs         # Admin routes: OAuth, JWT middleware, user CRUD, stats, share creation
  config/mod.rs    # Config struct loaded from environment variables
  error/mod.rs     # AppError enum with HTTP status mapping and JSON responses
  progress.rs      # ProgressReader wrapping AsyncRead, broadcast channel, download tracking
  file_indexer.rs  # Background OS thread scanning the data directory periodically
  worker/
    mod.rs         # TaskManager: task creation, status, SQLite persistence
    tasks.rs       # TaskWorker: processes tasks; create_7z_archive* helpers

migrations/        # SQLx migration files (run automatically at startup)
templates/         # Askama HTML templates (list_files.html, 404.html)
static/css/        # Tailwind CSS input (input.css)
dist/              # Compiled CSS output, images, HTML fragments — served at /assets
tests/
  common/mod.rs    # TestContext, DB seeding helpers
  integration_test.rs
```

## App State

`App` (cloned per request via Axum `State`):
- `db_pool` — SQLite connection pool
- `progress_channel_sender` — broadcast sender for download progress events
- `task_manager` — async task queue
- `indexer` — file system indexer
- `config` — loaded from env at startup
- `pending_auths` — in-memory map for PKCE/nonce during OAuth flow

## HTTP Routes

**Public:**
- `GET /files/:share_id` — download page (Askama template)
- `GET /files/:share_id/:filename` — file download (range requests supported)
- `HEAD /files/:share_id/:filename` — file metadata
- `GET /assets/*` — static assets (Tower `ServeDir`)

**Admin (`/admin`, Bearer JWT required):**
- `GET /admin/auth/google` — initiate OAuth PKCE flow
- `GET /admin/auth/google/callback` — OAuth callback, issues JWT
- `GET /admin/users` — list users
- `POST /admin/users` — create user
- `GET /admin/users/:id` — get user
- `DELETE /admin/users/:id` — delete user
- `GET /admin/stats/downloads` — download stats
- `GET /admin/stats/downloads/period` — stats by time period
- `GET /admin/stats/status` — status distribution
- `GET /admin/stats/recent` — recent downloads
- `POST /admin/shares` — create share link

## Database Schema

**`share_links`** — `id` (nanoid, PK), `expiration` (unix ts, -1 = none), `created_at`
**`share_link_files`** — join table: `share_link_id` → `files.id`
**`files`** — `id`, `info`, `file_size`, `sha256`, `path`
**`download`** — `id`, `file_path`, `ip_address`, `transaction_id`, `status`, `file_size`, `started_at`, `finished_at`
**`tasks`** — `id` (UUID), `task_type`, `status`, `input_data` (JSON), `output_data`, `progress`, timestamps
**`admin_users`** — `id`, `email`, `google_id`, `created_at`

## Configuration (Environment Variables)

**Required:**
- `JWT_SECRET` — min 32 characters
- `GOOGLE_CLIENT_ID`
- `GOOGLE_CLIENT_SECRET`

**Optional (defaults):**
- `HARDWIRE_HOST` (default: `http://localhost:8080`)
- `HARDWIRE_PORT` (default: `8080`)
- `HARDWIRE_DATA_DIR` (default: `./data`)
- `HARDWIRE_DB_PATH` (default: `./data/db.sqlite`)
- `HARDWIRE_DB_MAX_CONNECTIONS` (default: `10`)
- `HARDWIRE_DB_MIN_CONNECTIONS` (default: `2`)
- `HARDWIRE_DB_ACQUIRE_TIMEOUT` (default: `30`)
- `JWT_EXPIRY_HOURS` (default: `24`)
- `GOOGLE_REDIRECT_URL` (default: `http://localhost:8080/admin/auth/google/callback`)
- `HARDWIRE_MAX_FILE_SIZE_MB` (default: `5120`)
- `HARDWIRE_MAX_FILES_PER_SHARE` (default: `100`)
- `HARDWIRE_RATE_LIMIT_RPM` (default: `60`)
- `HARDWIRE_FILE_INDEXER_INTERVAL` (default: `300` seconds)
- `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`
- `OTEL_SERVICE_NAME` (default: `hardwire`)
- `TOKIO_CONSOLE` (default: `false`)

## Build

```sh
# CSS
npx @tailwindcss/cli -i ./static/css/input.css -o ./dist/css/output.css

# Database migrations + sqlx offline cache
export DATABASE_URL=sqlite://data/db.sqlite
sqlx migrate run --source migrations
cargo sqlx prepare

# Release binary
cargo build --release

# All at once
make all
```

**Docker:**
```sh
make push    # builds linux/amd64 image, pushes to Docker Hub
make deploy  # ssh into 'orion', docker compose pull + up -d
```

## SQLx Offline Mode

Compile-time query macros require either a live `DATABASE_URL` or the offline cache. The cache is at `sqlx-data.json` and `.sqlx/`. Run `cargo sqlx prepare` after schema changes.

## Error Handling

All errors flow through `AppError` in `src/error/mod.rs`. It implements `IntoResponse` and maps to appropriate HTTP status codes. Internal errors are logged but never exposed to clients; all responses return structured JSON `{ "error", "details", "code" }`.

## Tests

Run with `cargo test`. Integration tests in `tests/` spin up a real SQLite DB in a `TempDir` and run all migrations. Use `sqlx::query(...)` (runtime, not macro) in tests to avoid needing `DATABASE_URL`.

```sh
cargo test
```
