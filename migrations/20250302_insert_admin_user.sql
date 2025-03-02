-- Insert admin user with specific Google ID
INSERT INTO admin_users (email, google_id, created_at)
VALUES (
    'pestouille@gmail.com',
    '105153740956852908216',
    strftime('%s', 'now') -- Current Unix timestamp
);