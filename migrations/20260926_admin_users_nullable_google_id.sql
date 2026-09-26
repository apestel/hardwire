-- google_id becomes nullable: a pre-authorized admin (created by email, not
-- yet logged in) has no Google ID. The previous placeholder '' collided with
-- the UNIQUE constraint as soon as two admins were pending. SQLite cannot drop
-- NOT NULL in place, hence the table rebuild. UNIQUE allows several NULLs.
CREATE TABLE admin_users_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    email TEXT NOT NULL UNIQUE,
    google_id TEXT UNIQUE,
    created_at INTEGER NOT NULL
);
INSERT INTO admin_users_new (id, email, google_id, created_at)
    SELECT id, email, NULLIF(google_id, ''), created_at FROM admin_users;
DROP TABLE admin_users;
ALTER TABLE admin_users_new RENAME TO admin_users;
