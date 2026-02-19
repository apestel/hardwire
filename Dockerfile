# ── Stage 1: Build frontend ───────────────────────────────────────────────────
FROM node:22-slim AS frontend-build
WORKDIR /hardwire/frontend
COPY ./frontend/package*.json ./
RUN npm ci
COPY ./frontend ./
RUN npm run build
# Output: /hardwire/dist/admin/

# ── Stage 2: Build Rust binary ────────────────────────────────────────────────
FROM rust:slim-bookworm AS cargo-build
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config musl-tools libssl-dev librust-openssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /hardwire

# Cache dependency compilation — only invalidated when Cargo.toml/Cargo.lock change
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs \
    && cargo build --release \
    && rm -rf src

# Build the real binary
COPY .sqlx ./.sqlx
COPY src ./src
COPY migrations ./migrations
COPY templates ./templates
# Touch main.rs so cargo knows to re-link after the dummy build above
RUN touch src/main.rs && cargo build --release

# ── Stage 3: Runtime image ────────────────────────────────────────────────────
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    openssl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=cargo-build /hardwire/target/release/hardwire ./hardwire
COPY --from=frontend-build /hardwire/dist/admin ./dist/admin
COPY ./static ./static
COPY ./dist/css ./dist/css
COPY ./migrations ./migrations
EXPOSE 8080
CMD ["./hardwire", "-s"]
