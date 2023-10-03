CREATE TABLE share_links (
    id TEXT PRIMARY KEY NOT NULL,
    expiration INT NOT NULL,
    created_at INT NOT NULL
);

CREATE TABLE share_link_files (
    share_link_id TEXT,
    file_id INT
);

CREATE TABLE files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    info TEXT,
    file_size BIGINT,
    sha256 TEXT,
    path TEXT NOT NULL
);

CREATE TABLE download (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT,
    ip_address TEXT,
    transaction_id TEXT,
    status TEXT,
    file_size INT,
    started_at INT,
    finished_at INT
);