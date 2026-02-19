# Hardwire — CLAUDE.md

Self-hosted file sharing service (WetTransfer-like), written in Rust. Admins create share links via an admin SPA; recipients download via a public page.

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
| Admin frontend | SvelteKit 2 + Svelte 5 (runes) + Tailwind CSS v4 + Chart.js |
| CSS (public) | Tailwind CSS (compiled via `npx @tailwindcss/cli`) |
| CLI | clap 4 (derive) |

## Project Structure

```
src/
  main.rs          # Entry point, App state, router, download handlers, templates
  lib.rs           # Re-exports config and error modules for integration tests
  admin.rs         # Admin routes: OAuth, JWT middleware, user CRUD, stats, tasks, WS
  config/mod.rs    # Config struct loaded from environment variables
  error/mod.rs     # AppError enum with HTTP status mapping and JSON responses
  progress.rs      # ProgressReader wrapping AsyncRead, broadcast channel, download tracking
  file_indexer.rs  # Background OS thread scanning the data directory periodically
  worker/
    mod.rs         # TaskManager: task creation, status, SQLite persistence
    tasks.rs       # TaskWorker: processes tasks; create_7z_archive* helpers

frontend/          # SvelteKit admin SPA (built to dist/admin/, served at /admin)
  src/
    lib/
      api.ts       # Typed fetch wrappers (auto Bearer header)
      auth.ts      # JWT storage/validation (localStorage)
      types.ts     # TypeScript interfaces mirroring Rust structs
      stores/notifications.ts  # Svelte writable store for toast notifications
      components/  # StatsCard, DownloadsChart, FileTree, FileTreeNode,
                   # NotificationBar, Notification, RecentDownloadsTable, PeriodSelector
    routes/
      +layout.ts/.svelte        # Auth guard + nav + NotificationBar
      +page.svelte              # Login (Google OAuth button)
      auth/done/                # Receives ?token= from backend redirect, stores JWT
      auth/google/callback/     # Legacy callback page (kept for fallback)
      dashboard/                # Stats cards, Chart.js bar chart, recent downloads table
      files/                    # File tree (sort, multi-select, share links, 7z archive)

migrations/        # SQLx migration files (run automatically at startup)
templates/         # Askama HTML templates (list_files.html, 404.html)
static/css/        # Tailwind CSS input (input.css)
dist/              # Compiled CSS output + images — served at /assets
dist/admin/        # Built SvelteKit SPA — served at /admin (gitignored)
.sqlx/             # sqlx offline query cache (committed, updated via `cargo sqlx prepare`)
.github/workflows/
  ci.yml           # Build + test on push to main/wip and PRs
  release.yml      # Build Docker image, push to Hub, deploy via SSH on v* tags
tests/
  common/mod.rs    # TestContext, DB seeding helpers
  integration_test.rs
```

## App State

`App` (cloned per request via Axum `State<App>`):
- `db_pool` — SQLite connection pool
- `progress_channel_sender` — broadcast sender for download progress events
- `task_manager` — async task queue (Arc)
- `indexer` — file system indexer (background thread + signal channel)
- `config` — loaded from env at startup
- `pending_auths` — in-memory `HashMap<csrf_state, (Nonce, PkceCodeVerifier)>` for OAuth flow

**Important:** Custom extractors that need state must use `App: FromRef<S>` + `App::from_ref(state)`, NOT `parts.extensions.get::<App>()`. Axum does not put `.with_state()` state into request extensions.

## HTTP Routes

**Public:**
- `GET /s/:share_id` — download page (Askama template)
- `GET /s/:share_id/:file_id` — file download (range requests supported)
- `HEAD /s/:share_id/:file_id` — file metadata
- `GET /assets/*` — static assets (Tower `ServeDir`)
- `GET /admin/*` — SPA fallback (Tower `ServeDir` → `dist/admin/index.html`)

**Admin (`/admin`, Bearer JWT required except auth routes):**
- `GET  /admin/auth/google/login` — initiate OAuth PKCE flow (redirects to Google)
- `GET  /admin/auth/google/callback` — OAuth callback, issues JWT, redirects to `/admin/auth/done?token=`
- `GET  /admin/live_update?token=` — WebSocket, forwards download progress events as JSON
- `GET  /admin/api/users` — list admin users
- `POST /admin/api/users` — create admin user
- `GET  /admin/api/users/:id` — get user
- `DELETE /admin/api/users/:id` — delete user
- `POST /admin/api/tasks` — create async task (e.g. CreateArchive)
- `GET  /admin/api/tasks/:id` — get task status + archive_path
- `GET  /admin/api/tasks/:id/download` — stream completed archive file
- `GET  /admin/api/list_files` — list indexed files (from FileIndexer)
- `POST /admin/api/files/rescan` — trigger immediate file rescan
- `POST /admin/api/create_shared_link` — create share link, returns public URL
- `GET  /admin/api/stats/downloads` — aggregate download stats
- `GET  /admin/api/stats/downloads/by_period?period=&limit=` — downloads over time
- `GET  /admin/api/stats/downloads/recent?limit=` — recent download records
- `GET  /admin/api/stats/downloads/status` — status distribution

## Database Schema

**`share_links`** — `id` (nanoid, PK), `expiration` (unix ts, -1 = none), `created_at`
**`share_link_files`** — join table: `share_link_id` → `files.id`
**`files`** — `id`, `info`, `file_size`, `sha256`, `path`
**`download`** — `id`, `file_path`, `ip_address`, `transaction_id`, `status`, `file_size`, `started_at`, `finished_at`
**`tasks`** — `id` (UUID), `task_type`, `status` (Pending/Running/Completed/Failed), `input_data` (JSON), `output_data` (JSON, contains `archive_path`), `progress` (0–100), timestamps
**`admin_users`** — `id`, `email`, `google_id`, `created_at`

## TaskInput Serialization

`TaskInput` uses `#[serde(tag = "type", content = "data")]`. From the frontend:
```json
{ "type": "CreateArchive", "data": { "files": [...], "output_path": "archive-123" } }
```
`output_path` is resolved relative to `HARDWIRE_DATA_DIR` in `TaskWorker`.

## TaskStatus Serialization

`TaskStatus` serializes as **PascalCase** (`"Pending"`, `"Running"`, `"Completed"`, `"Failed"`). The frontend `types.ts` uses these exact strings.

## Admin OAuth Flow

1. Browser → `GET /admin/auth/google/login` — generates PKCE challenge, stores `(Nonce, PkceCodeVerifier)` in `pending_auths`, redirects to Google
2. Google → `GET /admin/auth/google/callback?code=&state=` — exchanges code (with PKCE verifier), verifies ID token, upserts `admin_users`, issues JWT, redirects to `/admin/auth/done?token=<jwt>`
3. SPA `/auth/done` page — reads `?token=`, stores in localStorage, redirects to `/admin/dashboard`

## Configuration (Environment Variables)

**Required:**
- `JWT_SECRET` — min 32 characters
- `GOOGLE_CLIENT_ID`
- `GOOGLE_CLIENT_SECRET`

**Optional (defaults):**
- `HARDWIRE_HOST` (default: `http://localhost:8080`)
- `HARDWIRE_PORT` (default: `8080`)
- `HARDWIRE_DATA_DIR` (default: `./data`) — root of shared/indexed files and 7z archive output
- `HARDWIRE_DB_PATH` (default: `./data/db.sqlite`) — SQLite database path (independent of data_dir)
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
# Public CSS
npx @tailwindcss/cli -i ./static/css/input.css -o ./dist/css/output.css

# Frontend SPA
cd frontend && npm install && npm run build   # outputs to dist/admin/

# Database migrations + sqlx offline cache
export DATABASE_URL=sqlite://data/db.sqlite
sqlx migrate run --source migrations
cargo sqlx prepare   # must re-run after any sqlx::query! changes

# Release binary
cargo build --release

# All at once
make all
```

**Docker:**
```sh
make push            # build linux/amd64 image tagged with git version, push to Hub
make deploy          # SSH into 'orion', docker compose pull + up hardwire
make tag V=1.2.3     # create+push git tag → triggers GitHub Actions release pipeline
```

## CI/CD (GitHub Actions)

- **ci.yml** — runs on push/PR: Rust build+test, sqlx cache check, frontend type-check+build
- **release.yml** — triggered by `v*` tags: builds Docker image (`:latest` + `:<version>` + `:<sha>`), pushes to Docker Hub with GHA layer cache, deploys via SSH

**Required secrets:** `DOCKERHUB_USERNAME`, `DOCKERHUB_TOKEN`, `DEPLOY_HOST`, `DEPLOY_USER`, `DEPLOY_SSH_KEY`

## SQLx Offline Mode

Compile-time query macros require either a live `DATABASE_URL` or the offline cache in `.sqlx/`. Always commit `.sqlx/` after schema or query changes. Run `DATABASE_URL=sqlite://data/db.sqlite cargo sqlx prepare` to regenerate.

## Error Handling

All errors flow through `AppError` in `src/error/mod.rs`. Maps to appropriate HTTP status codes. Internal errors are logged but never exposed to clients; responses return structured JSON `{ "error", "details", "code" }`.

## Tests

```sh
cargo test
```

Integration tests in `tests/` spin up a real SQLite DB in a `TempDir` and run all migrations. Use `sqlx::query(...)` (runtime, not macro) in tests to avoid needing `DATABASE_URL`.
