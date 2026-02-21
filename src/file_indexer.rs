use chrono::Utc;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use tokio::sync::oneshot;

#[derive(Serialize, Debug, Clone)]
pub struct FileInfo {
    name: String,
    full_path: String,
    is_dir: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    modified_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    children: Option<Vec<FileInfo>>,
}

/// Signal sent to the indexer thread: rescan now, then optionally notify the caller.
struct RescanSignal {
    done_tx: Option<oneshot::Sender<()>>,
}

#[derive(Clone, Debug)]
pub struct FileIndexer {
    pub files: Arc<Mutex<Option<Vec<FileInfo>>>>,
    /// Flat map: absolute_path → file_size, rebuilt on every scan.
    pub path_cache: Arc<Mutex<HashMap<String, u64>>>,
    signal_tx: Sender<RescanSignal>,
}

impl FileIndexer {
    pub fn new(base_path: &Path, update_interval: u64) -> FileIndexer {
        let (tx, rx) = mpsc::channel::<RescanSignal>();
        let rescan_tx = tx.clone();
        let base_path: Arc<PathBuf> = Arc::new(base_path.to_path_buf());

        let files: Arc<Mutex<Option<Vec<FileInfo>>>> = Arc::new(Mutex::new(Some(vec![])));
        let files_clone = Arc::clone(&files);
        let path_cache: Arc<Mutex<HashMap<String, u64>>> = Arc::new(Mutex::new(HashMap::new()));
        let path_cache_clone = Arc::clone(&path_cache);
        let base_path_clone = Arc::clone(&base_path);

        thread::spawn(move || {
            let do_scan = |done_tx: Option<oneshot::Sender<()>>| {
                match rec_scan_dir(&base_path_clone, &base_path_clone) {
                    Ok(dir_structure) => {
                        let mut cache = HashMap::new();
                        collect_file_sizes(&dir_structure, &base_path_clone, &mut cache);
                        let mut output = files_clone.lock().unwrap();
                        *output = Some(dir_structure);
                        let mut pc = path_cache_clone.lock().unwrap();
                        *pc = cache;
                    }
                    Err(e) => eprintln!("Error scanning directory: {}", e),
                }
                if let Some(tx) = done_tx {
                    let _ = tx.send(());
                }
            };

            // Initial scan on startup
            do_scan(None);

            loop {
                // Wait for either a manual signal or the periodic interval
                match rx.recv_timeout(Duration::from_secs(update_interval)) {
                    Ok(signal) => {
                        println!("Manual rescan signal received at {}", Utc::now());
                        do_scan(signal.done_tx);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        do_scan(None);
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });

        FileIndexer {
            files,
            path_cache,
            signal_tx: rescan_tx,
        }
    }

    /// Trigger a rescan and return a receiver that resolves when the scan completes.
    pub fn rescan_and_wait(&self) -> Option<oneshot::Receiver<()>> {
        let (done_tx, done_rx) = oneshot::channel();
        match self.signal_tx.send(RescanSignal { done_tx: Some(done_tx) }) {
            Ok(()) => Some(done_rx),
            Err(_) => None,
        }
    }

    /// Look up the cached file size for an absolute path. Returns `None` on cache miss.
    pub fn get_file_size(&self, abs_path: &str) -> Option<u64> {
        self.path_cache.lock().ok()?.get(abs_path).copied()
    }
}

/// Walk the FileInfo tree and populate `cache` with absolute_path → size for all files.
fn collect_file_sizes(
    entries: &[FileInfo],
    base_path: &Path,
    cache: &mut HashMap<String, u64>,
) {
    for entry in entries {
        if entry.is_dir {
            if let Some(children) = &entry.children {
                collect_file_sizes(children, base_path, cache);
            }
        } else if let Some(size) = entry.size {
            let abs = base_path.join(&entry.full_path).to_string_lossy().into_owned();
            cache.insert(abs, size);
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
            let modified_at = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64);
            let created_at = metadata
                .created()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64);

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
                modified_at,
                created_at,
                children,
            });
        }
    }

    Ok(files_info)
}
