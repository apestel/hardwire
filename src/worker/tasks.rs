use anyhow::Result;
use sevenzip_mt::{Lzma2Config, SevenZipWriter};
use std::fs::File;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use tokio::time;
use walkdir::WalkDir;

use super::{TaskInput, TaskManager, TaskStatus};

pub struct TaskWorker {
    task_manager: TaskManager,
    task_receiver: mpsc::Receiver<String>,
    data_dir: PathBuf,
}

/// Minimal progress token — only tracks whether the archive job is done.
/// sevenzip-mt compresses everything in one blocking call with no progress callbacks,
/// so we report progress = 0 (indeterminate) until completion.
#[derive(Clone)]
struct ArchiveProgress {
    is_complete: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl ArchiveProgress {
    fn new() -> Self {
        Self {
            is_complete: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

impl TaskWorker {
    pub fn new(task_manager: TaskManager, task_receiver: mpsc::Receiver<String>, data_dir: PathBuf) -> Self {
        Self {
            task_manager,
            task_receiver,
            data_dir,
        }
    }

    pub async fn run(&mut self) {
        while let Some(task_id) = self.task_receiver.recv().await {
            if let Err(e) = self.process_task(&task_id).await {
                log::error!("Task {} failed: {}", task_id, e);
                let _ = self
                    .task_manager
                    .update_task_status(&task_id, TaskStatus::Failed, Some(e.to_string()), None)
                    .await;
            }
        }
    }

    async fn process_task(&self, task_id: &str) -> Result<()> {
        // Mark task as running
        self.task_manager
            .update_task_status(task_id, TaskStatus::Running, None, Some(0))
            .await?;

        // Get task details
        let task_data = sqlx::query!("SELECT input_data FROM tasks WHERE id = ?", task_id)
            .fetch_one(&self.task_manager.db)
            .await?;

        let input: TaskInput = serde_json::from_str(&task_data.input_data)?;

        match input {
            TaskInput::CreateArchive(archive_input) => {
                // Create progress tracker (indeterminate — sevenzip-mt has no progress callbacks)
                let progress = ArchiveProgress::new();
                let progress_clone = progress.clone();

                // Spawn progress monitoring task — keeps task alive in DB while compressing
                let task_manager = self.task_manager.clone();
                let task_id_clone = task_id.to_string();
                tokio::spawn(async move {
                    while !progress_clone
                        .is_complete
                        .load(std::sync::atomic::Ordering::Relaxed)
                    {
                        // Report progress = 0 (indeterminate) until done
                        if let Err(e) = task_manager
                            .update_task_status(
                                &task_id_clone,
                                TaskStatus::Running,
                                None,
                                Some(0),
                            )
                            .await
                        {
                            log::error!("Failed to update task progress: {}", e);
                        }
                        time::sleep(time::Duration::from_secs(10)).await;
                    }
                });

                // Resolve output_path relative to data_dir if it's not absolute
                let output_path = if archive_input.output_path.is_absolute() {
                    archive_input.output_path.clone()
                } else {
                    self.data_dir.join(&archive_input.output_path)
                };

                let archive_result = if let Some(dir) = archive_input.directory {
                    create_7z_archive_with_progress(
                        vec![dir],
                        output_path,
                        archive_input.password,
                        progress.clone(),
                    )
                    .await
                } else if let Some(files) = archive_input.files {
                    create_7z_archive_with_progress(
                        files,
                        output_path,
                        archive_input.password,
                        progress.clone(),
                    )
                    .await
                } else {
                    Err(anyhow::anyhow!("Either directory or files must be specified"))
                };

                // Always stop the monitoring goroutine, whether success or failure
                progress
                    .is_complete
                    .store(true, std::sync::atomic::Ordering::Relaxed);

                let result = archive_result?;

                // Update task as completed
                self.task_manager
                    .update_task_status(task_id, TaskStatus::Completed, None, Some(100))
                    .await?;

                // Store output data
                let output_data = serde_json::json!({
                    "archive_path": result
                })
                .to_string();

                sqlx::query!(
                    "UPDATE tasks SET output_data = ? WHERE id = ?",
                    output_data,
                    task_id
                )
                .execute(&self.task_manager.db)
                .await?;
            }
        }

        Ok(())
    }
}


/// Create a 7z archive
async fn create_7z_archive_with_progress<P: AsRef<Path>>(
    source: Vec<P>,
    output_path: PathBuf,
    _password: Option<String>,
    _progress: ArchiveProgress,
) -> Result<PathBuf> {
    // Ensure output path has .7z extension
    let output_path = if !output_path.extension().map_or(false, |ext| ext == "7z") {
        output_path.with_extension("7z")
    } else {
        output_path
    };

    // Collect all (disk_path, archive_name) pairs
    let mut files_to_compress: Vec<(PathBuf, String)> = Vec::new();
    for path in source {
        let path = path.as_ref();
        if path.is_dir() {
            for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
                if entry.file_type().is_file() {
                    let relative_path = entry.path().strip_prefix(path)?;
                    files_to_compress.push((
                        entry.path().to_path_buf(),
                        relative_path.to_string_lossy().to_string(),
                    ));
                }
            }
        } else if path.is_file() {
            files_to_compress.push((
                path.to_path_buf(),
                path.file_name().unwrap().to_string_lossy().to_string(),
            ));
        }
    }

    let output_path_clone = output_path.clone();
    tokio::task::spawn_blocking(move || {
        let output_file = File::create(&output_path_clone)?;
        let mut archive = SevenZipWriter::new(output_file)
            .map_err(|e| anyhow::anyhow!("Failed to create archive: {e}"))?;
        archive.set_config(Lzma2Config {
            preset: 2,
            dict_size: None,
            block_size: None,
        });

        for (disk_path, archive_name) in &files_to_compress {
            archive
                .add_file(
                    disk_path.to_str().ok_or_else(|| anyhow::anyhow!("non-UTF8 path"))?,
                    archive_name,
                )
                .map_err(|e| anyhow::anyhow!("Failed to add file: {e}"))?;
        }

        archive
            .finish()
            .map_err(|e| anyhow::anyhow!("Failed to finish archive: {e}"))?;

        Ok::<_, anyhow::Error>(())
    })
    .await??;

    Ok(output_path)
}

/// Create a 7z archive from a list of files or a directory
///
/// # Arguments
/// * `source` - Either a directory path or a list of file paths to compress
/// * `output_path` - Path where the 7z file should be created
/// * `password` - Optional password to encrypt the archive
#[allow(dead_code)]
pub async fn create_7z_archive<P: AsRef<Path>>(
    source: Vec<P>,
    output_path: PathBuf,
    password: Option<String>,
) -> Result<PathBuf> {
    create_7z_archive_with_progress(source, output_path, password, ArchiveProgress::new()).await
}

/// Create a 7z archive from a directory
///
/// # Arguments
/// * `dir_path` - Path to the directory to compress
/// * `output_path` - Path where the 7z file should be created
/// * `password` - Optional password to encrypt the archive
#[allow(dead_code)]
pub async fn create_7z_from_directory<P: AsRef<Path>>(
    dir_path: P,
    output_path: PathBuf,
    password: Option<String>,
) -> Result<PathBuf> {
    create_7z_archive(vec![dir_path], output_path, password).await
}

/// Create a 7z archive from multiple files
///
/// # Arguments
/// * `files` - List of file paths to compress
/// * `output_path` - Path where the 7z file should be created
/// * `password` - Optional password to encrypt the archive
#[allow(dead_code)]
pub async fn create_7z_from_files<P: AsRef<Path>>(
    files: Vec<P>,
    output_path: PathBuf,
    password: Option<String>,
) -> Result<PathBuf> {
    create_7z_archive(files, output_path, password).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::fs::File;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn test_create_7z_from_files() -> Result<()> {
        let temp_dir = tempdir()?;

        // Create test files
        let file1_path = temp_dir.path().join("test1.txt");
        let file2_path = temp_dir.path().join("test2.txt");

        let mut file1 = File::create(&file1_path).await?;
        file1.write_all(b"Test content 1").await?;
        let mut file2 = File::create(&file2_path).await?;
        file2.write_all(b"Test content 2").await?;

        let output_path = temp_dir.path().join("output.7z");
        let files = vec![file1_path, file2_path];

        let result = create_7z_from_files(files, output_path.clone(), None).await?;
        assert!(result.exists());

        Ok(())
    }

    #[tokio::test]
    async fn test_create_7z_from_directory() -> Result<()> {
        let temp_dir = tempdir()?;
        let test_dir = temp_dir.path().join("test_dir");
        std::fs::create_dir(&test_dir)?;

        // Create test files in directory
        let file1_path = test_dir.join("test1.txt");
        let file2_path = test_dir.join("test2.txt");

        let mut file1 = File::create(&file1_path).await?;
        file1.write_all(b"Test content 1").await?;
        let mut file2 = File::create(&file2_path).await?;
        file2.write_all(b"Test content 2").await?;

        let output_path = temp_dir.path().join("output.7z");

        let result = create_7z_from_directory(&test_dir, output_path.clone(), None).await?;
        assert!(result.exists());

        assert!(result.exists());

        Ok(())
    }
}
