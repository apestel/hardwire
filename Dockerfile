# Stage 1: Build frontend
FROM node:22-slim AS frontend-build
WORKDIR /hardwire/frontend
COPY ./frontend/package*.json ./
RUN npm ci
COPY ./frontend ./
RUN npm run build
# Output: /hardwire/dist/admin/ (via outDir: '../dist/admin' in svelte.config.js)

# Stage 2: Build Rust binary
FROM rust:slim-bookworm AS cargo-build
RUN apt-get update
RUN apt-get install pkg-config musl-tools libssl-dev librust-openssl-dev -y
WORKDIR /hardwire
COPY ./Cargo.toml ./Cargo.toml
COPY ./Cargo.lock ./Cargo.lock
COPY ./.sqlx ./.sqlx
COPY ./src ./src
COPY ./migrations ./migrations
COPY ./templates ./templates
COPY ./sqlx-data.json ./sqlx-data.json
RUN cargo build --release

# Stage 3: Runtime image
FROM debian:bookworm-slim
RUN apt-get update
RUN apt-get install openssl ca-certificates -y
WORKDIR /app
COPY --from=cargo-build /hardwire/target/release/hardwire /app/hardwire
COPY ./static ./static
COPY ./dist ./dist
COPY --from=frontend-build /hardwire/dist/admin /app/dist/admin
COPY ./migrations ./migrations
EXPOSE 8080
CMD ["./hardwire", "-s"]
