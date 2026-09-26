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
    signal_tx: Sender<RescanSignal>,
}

impl FileIndexer {
    pub fn new(base_path: &Path, update_interval: u64) -> FileIndexer {
        let (tx, rx) = mpsc::channel::<RescanSignal>();
        let rescan_tx = tx.clone();
        let base_path: Arc<PathBuf> = Arc::new(base_path.to_path_buf());

        let files: Arc<Mutex<Option<Vec<FileInfo>>>> = Arc::new(Mutex::new(Some(vec![])));
        let files_clone = Arc::clone(&files);
        let base_path_clone = Arc::clone(&base_path);

        thread::spawn(move || {
            let do_scan = |done_tx: Option<oneshot::Sender<()>>| {
                match rec_scan_dir(&base_path_clone, &base_path_clone) {
                    Ok(dir_structure) => {
                        let mut output = files_clone.lock().unwrap();
                        *output = Some(dir_structure);
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
}

/// Only an unreadable `path` itself is an error (the caller then keeps the
/// previous listing). A bad entry below it — dangling symlink, permission
/// denied, unreadable subdirectory — is logged and skipped, so one broken file
/// no longer freezes the whole listing.
fn rec_scan_dir(base_path: &Path, path: &Path) -> io::Result<Vec<FileInfo>> {
    let mut files_info = Vec::new();

    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    tracing::warn!(dir = %path.display(), "skipping unreadable entry: {e}");
                    continue;
                }
            };
            let path = entry.path();

            let metadata = match fs::metadata(&path) {
                Ok(metadata) => metadata,
                Err(e) => {
                    tracing::warn!(path = %path.display(), "skipping entry: {e}");
                    continue;
                }
            };
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
                Some(rec_scan_dir(base_path, &path).unwrap_or_else(|e| {
                    tracing::warn!(dir = %path.display(), "cannot list directory: {e}");
                    vec![]
                }))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn scan_skips_broken_entries() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("ok.txt"), b"x").unwrap();
        std::os::unix::fs::symlink("/nonexistent", dir.path().join("dangling")).unwrap();

        let files = rec_scan_dir(dir.path(), dir.path()).unwrap();
        let names: Vec<_> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["ok.txt"]);
    }
}
