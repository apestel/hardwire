# Hardwire

Hardwire is a toy project aiming to provide similar services to wetransfer.
In the past, I was using a self hosted Nextcloud docker instance which was very ressource consuming and quite slow to deliver static files.

The project is developed with Rust language and SQLite on server side and use Tailwind CSS.

Currently it lacks: 
- Unit/Integration tests and by extension Continous Integration
- Admin UI
- Usage stats
- Clean warnings
- Metadata associated to media files (crawled from TMDB for example)
- and many more...

Very basic, probably not production ready except if you're willing like me to have your hands dirty :)

    ./hardwire --help
    hardwire 0.1.0
    Adrien Pestel

    USAGE:
        hardwire [OPTIONS]

    OPTIONS:
        -f, --filename <FILENAME>    Filename to publish
        -h, --help                   Print help information
        -s, --server                 Server
        -V, --version                Print version information


## Create the first Admin user

1. First, run database migrations:
```bash
make db-migrate
# or
sqlx migrate run
```

2. Get your Google ID:
   - Go to [Google OAuth 2.0 Playground](https://developers.google.com/oauthplayground/)
   - Click on "OAuth 2.0 Configuration" in the top right
   - Select your OAuth credentials
   - Choose "Google OAuth2 API v2" from the list
   - Select "https://www.googleapis.com/auth/userinfo.profile" and "https://www.googleapis.com/auth/userinfo.email"
   - Click "Authorize APIs"
   - After authorization, click "Exchange authorization code for tokens"
   - Click "Step 3 - Configure request to API"
   - Make a GET request to https://www.googleapis.com/oauth2/v2/userinfo
   - In the response, you'll find your Google ID in the "id" field

3. Insert the first admin user in the SQLite Database:
```sql
INSERT INTO admin_users (email, google_id) 
VALUES ('your-email@gmail.com', 'your-google-id');
```

After this setup, you can use the admin API to manage other admin users.

| Environment variable | Default value         | Description                            |
|----------------------|-----------------------|----------------------------------------|
| HARDWIRE_HOST        | http://localhost:8080 | Base URI used to generate shared links |
| HARDWIRE_PORT        | 8080                  | Server listen port                     |
| OTEL_EXPORTER_OTLP_TRACES_PROTOCOL | http/protobuf | OpenTelemetry Traces Protocol |
| OTEL_EXPORTER_OTLP_TRACES_ENDPOINT | OTEL_EXPORTER_OTLP_ENDPOINT or http://localhost:4318 (protobuf) or http://localhost:4317 | Opentelemetry exporter endpoint |
| OTEL_RESOURCE_ATTRIBUTES | No default value | service.name=rust-app (you can name it whatever you want) |