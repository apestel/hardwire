-- Create tasks table
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY NOT NULL,
    task_type TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    started_at INTEGER,
    finished_at INTEGER,
    error TEXT,
    input_data TEXT NOT NULL,  -- JSON encoded input data
    output_data TEXT,          -- JSON encoded output data
    progress INTEGER DEFAULT 0  -- Progress percentage (0-100)
);
