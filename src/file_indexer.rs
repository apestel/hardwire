use chrono::Utc;
use serde::Serialize;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

#[derive(Serialize, Debug, Clone)]
pub struct FileInfo {
    name: String,
    full_path: String,
    is_dir: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    children: Option<Vec<FileInfo>>,
}

#[derive(Clone, Debug)]
pub struct FileIndexer {
    pub files: Arc<Mutex<Option<Vec<FileInfo>>>>,
    pub signal_index_updater: Sender<()>,
}

impl FileIndexer {
    pub fn new(base_path: &Path, update_interval: u64) -> FileIndexer {
        let (tx, rx) = mpsc::channel();
        let rescan_tx = tx.clone();
        let base_path: Arc<PathBuf> = Arc::new(base_path.to_path_buf());

        let files: Arc<Mutex<Option<Vec<FileInfo>>>> = Arc::new(Mutex::new(Some(vec![])));
        // Spawn a thread to run the scan periodically
        let files_clone = Arc::clone(&files);
        let base_path_clone = Arc::clone(&base_path);

        thread::spawn(move || {
            loop {
                match rec_scan_dir(&base_path_clone, &base_path_clone) {
                    Ok(dir_structure) => {
                        let mut output = files_clone.lock().unwrap();
                        *output = Some(dir_structure);
                    }
                    Err(e) => eprintln!("Error scanning directory: {}", e),
                }

                // Wait for either a minute or a manual rescan signal
                let res = rx.recv_timeout(Duration::from_secs(update_interval));
                if res.is_ok() {
                    println!("Manual rescan signal received at {}", Utc::now());
                }
            }
        });

        FileIndexer {
            files,
            signal_index_updater: rescan_tx.clone(),
        }
    }
}

fn rec_scan_dir(base_path: &Path, path: &Path) -> io::Result<Vec<FileInfo>> {
    let mut files_info = Vec::new();

    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();

            let metadata = fs::metadata(&path)?;
            let size = if path.is_file() {
                Some(metadata.len())
            } else {
                None
            };

            let name = path
                .file_name()
                .unwrap_or_else(|| path.as_os_str())
                .to_string_lossy()
                .into_owned();

            let full_path = path
                .strip_prefix(base_path)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();

            let children = if path.is_dir() {
                Some(rec_scan_dir(base_path, &path)?)
            } else {
                None
            };

            files_info.push(FileInfo {
                name,
                full_path,
                is_dir: path.is_dir(),
                size,
                children,
            });
        }
    }

    Ok(files_info)
}
