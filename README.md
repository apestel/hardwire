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

    hardwire --help
    hardwire 0.1.0
    Adrien Pestel

    USAGE:
        hardwire [OPTIONS]

    OPTIONS:
        -f, --filename <FILENAME>    Filename to publish
        -h, --help                   Print help information
        -s, --server                 Server
        -V, --version                Print version information


| Environment variable | Default value         | Description                            |
|----------------------|-----------------------|----------------------------------------|
| HARDWIRE_HOST        | http://localhost:8080 | Base URI used to generate shared links |
| HARDWIRE_PORT        | 8080                  | Server listen port                     |