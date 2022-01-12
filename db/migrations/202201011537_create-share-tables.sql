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

CREATE TABLE downloads (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ip_address TEXT,
    file_id INT,
    downloaded_at INT
);