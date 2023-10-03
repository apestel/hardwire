use std::time::SystemTime;

use chrono::{DateTime, Utc};
use indextree::Arena;
use serde::Serialize;
use sqlx::Sqlite;
use walkdir::WalkDir;

#[derive(Serialize)]
pub struct FileNode {
    is_directory: bool,
    name: String,
    created_at: i64,
    modified_at: i64,
}

pub struct Indexer {
    base_path: String,
    db_pool: sqlx::Pool<Sqlite>,
    file_tree: Arena<FileNode>,
}

impl Indexer {
    pub fn new(base_path: String, db_pool: sqlx::Pool<Sqlite>) -> Self {
        Self {
            base_path,
            db_pool,
            file_tree: Arena::new(),
        }
    }

    pub fn index(mut self) {
        for entry in WalkDir::new(self.base_path)
            .min_depth(1)
            .max_depth(2)
            .into_iter()
            .flatten()
        {
            if let Ok(metadata) = entry.metadata() {
                let modified_at = metadata
                    .modified()
                    .unwrap()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap();
                // Create DateTime from SystemTime
                let datetime = DateTime::<Utc>::from(metadata.modified().unwrap());
                // Formats the combined date and time with the specified format string.
                let timestamp_str = datetime.format("%Y-%m-%d %H:%M:%S.%f").to_string();
                println! {"{}",timestamp_str};
                println!(
                    "File: {}, modified_at: {}",
                    entry.file_name().to_str().unwrap(),
                    timestamp_str
                );
                let file_node = FileNode {
                    is_directory: metadata.is_dir(),
                    name: entry.file_name().to_str().unwrap().to_owned(),
                    created_at: datetime.timestamp(),
                    modified_at: datetime.timestamp(),
                };
                self.file_tree.new_node(file_node);
            }
        }
    }
}
