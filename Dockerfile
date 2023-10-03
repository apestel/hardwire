FROM rustdocker/rust:stable as cargo-build 
ENV PKG_CONFIG_ALLOW_CROSS=1
RUN apt-get update 
RUN apt-get install musl-tools libssl-dev -y 
RUN /root/.cargo/bin/rustup target add x86_64-unknown-linux-musl	 
RUN USER=root /root/.cargo/bin/cargo new --bin hardwire 
WORKDIR /hardwire 
COPY ./Cargo.toml ./Cargo.toml 
COPY ./Cargo.lock ./Cargo.lock
RUN rm src/*.rs 
COPY ./.sqlx ./.sqlx
COPY ./src ./src 
COPY ./db ./db
COPY ./templates ./templates 
COPY ./sqlx-data.json ./sqlx-data.json 
RUN /root/.cargo/bin/cargo build --release --target=x86_64-unknown-linux-musl

FROM alpine:latest 
WORKDIR /app
COPY --from=cargo-build /hardwire/target/x86_64-unknown-linux-musl/release/hardwire /app/hardwire
COPY ./static ./static
COPY ./dist ./dist
COPY ./db ./db 
EXPOSE 8080
CMD ["./hardwire", "-s"]