use anyhow::Result;
use sevenz_rust::{self, SevenZArchiveEntry};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use tokio::time;
use walkdir::WalkDir;

use super::{TaskInput, TaskManager, TaskStatus};

pub struct TaskWorker {
    task_manager: TaskManager,
    task_receiver: mpsc::Receiver<String>,
}

#[derive(Clone)]
struct ArchiveProgress {
    total_bytes: std::sync::Arc<std::sync::atomic::AtomicU64>,
    processed_bytes: std::sync::Arc<std::sync::atomic::AtomicU64>,
    is_complete: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl ArchiveProgress {
    fn new(total_bytes: u64) -> Self {
        Self {
            total_bytes: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(total_bytes)),
            processed_bytes: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            is_complete: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn get_progress_percentage(&self) -> i32 {
        let processed = self
            .processed_bytes
            .load(std::sync::atomic::Ordering::Relaxed);
        let total = self.total_bytes.load(std::sync::atomic::Ordering::Relaxed);
        if total == 0 {
            return 0;
        }
        ((processed as f64 / total as f64) * 100.0) as i32
    }
}

impl TaskWorker {
    pub fn new(task_manager: TaskManager, task_receiver: mpsc::Receiver<String>) -> Self {
        Self {
            task_manager,
            task_receiver,
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
                // Calculate total size of files to compress
                let mut total_size = 0u64;
                if let Some(dir) = &archive_input.directory {
                    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
                        if entry.file_type().is_file() {
                            if let Ok(metadata) = entry.metadata() {
                                total_size += metadata.len();
                            }
                        }
                    }
                } else if let Some(files) = &archive_input.files {
                    for file in files {
                        if let Ok(metadata) = std::fs::metadata(file) {
                            total_size += metadata.len();
                        }
                    }
                }

                // Create progress tracker
                let progress = ArchiveProgress::new(total_size);
                let progress_clone = progress.clone();

                // Spawn progress monitoring task
                let task_manager = self.task_manager.clone();
                let task_id_clone = task_id.to_string();
                tokio::spawn(async move {
                    while !progress_clone
                        .is_complete
                        .load(std::sync::atomic::Ordering::Relaxed)
                    {
                        let progress_percentage = progress_clone.get_progress_percentage();
                        if let Err(e) = task_manager
                            .update_task_status(
                                &task_id_clone,
                                TaskStatus::Running,
                                None,
                                Some(progress_percentage),
                            )
                            .await
                        {
                            log::error!("Failed to update task progress: {}", e);
                        }
                        time::sleep(time::Duration::from_secs(10)).await;
                    }
                });

                let result = if let Some(dir) = archive_input.directory {
                    create_7z_archive_with_progress(
                        vec![dir],
                        archive_input.output_path,
                        archive_input.password,
                        progress.clone(),
                    )
                    .await?
                } else if let Some(files) = archive_input.files {
                    create_7z_archive_with_progress(
                        files,
                        archive_input.output_path,
                        archive_input.password,
                        progress.clone(),
                    )
                    .await?
                } else {
                    anyhow::bail!("Either directory or files must be specified");
                };

                // Mark progress as complete
                progress
                    .is_complete
                    .store(true, std::sync::atomic::Ordering::Relaxed);

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

/// A reader that tracks the number of bytes read
struct ProgressReader<R: Read> {
    inner: R,
    progress: ArchiveProgress,
}

impl<R: Read> ProgressReader<R> {
    fn new(inner: R, progress: ArchiveProgress) -> Self {
        Self { inner, progress }
    }
}

impl<R: Read> Read for ProgressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.progress
                .processed_bytes
                .fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(n)
    }
}

/// Create a 7z archive with progress tracking
async fn create_7z_archive_with_progress<P: AsRef<Path>>(
    source: Vec<P>,
    output_path: PathBuf,
    password: Option<String>,
    progress: ArchiveProgress,
) -> Result<PathBuf> {
    // Ensure output path has .7z extension
    let output_path = if !output_path.extension().map_or(false, |ext| ext == "7z") {
        output_path.with_extension("7z")
    } else {
        output_path
    };

    // Create the output file
    let output_file = File::create(&output_path)?;
    let writer = BufWriter::new(output_file);

    // Collect all files to compress
    let mut files_to_compress = Vec::new();
    for path in source {
        let path = path.as_ref();
        if path.is_dir() {
            // If it's a directory, walk through it recursively
            for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
                if entry.file_type().is_file() {
                    let relative_path = entry.path().strip_prefix(path)?;
                    files_to_compress
                        .push((entry.path().to_path_buf(), relative_path.to_path_buf()));
                }
            }
        } else if path.is_file() {
            // If it's a file, add it directly
            files_to_compress.push((path.to_path_buf(), path.file_name().unwrap().into()));
        }
    }

    // Create archive with collected files
    tokio::task::spawn_blocking(move || {
        let mut archive = sevenz_rust::SevenZWriter::new(writer)?;
        // Compression methods should be set to COPY to avoid performance penalty. Unfortunately it's not supported yet.
        // LZMA2 is in used but should support multithreading in the future to perform better (quite slow right now)
        let mut compression_methods = vec![sevenz_rust::SevenZMethodConfiguration::from(
            // sevenz_rust::SevenZMethod::COPY,
            sevenz_rust::SevenZMethod::LZMA2,
        )];
        if let Some(pass) = password {
            compression_methods.push(sevenz_rust::SevenZMethodConfiguration::from(
                sevenz_rust::AesEncoderOptions::new(sevenz_rust::Password::from(pass.as_str())),
            ));
        }
        archive.set_content_methods(compression_methods);
        for (file_path, name) in files_to_compress {
            let file = File::open(&file_path)?;
            let reader = BufReader::new(file);
            let progress_reader = ProgressReader::new(reader, progress.clone());

            archive.push_archive_entry(
                SevenZArchiveEntry::from_path(&file_path, name.to_string_lossy().to_string()),
                Some(progress_reader),
            )?;
        }

        archive.finish()?;
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
pub async fn create_7z_archive<P: AsRef<Path>>(
    source: Vec<P>,
    output_path: PathBuf,
    password: Option<String>,
) -> Result<PathBuf> {
    create_7z_archive_with_progress(source, output_path, password, ArchiveProgress::new(0)).await
}

/// Create a 7z archive from a directory
///
/// # Arguments
/// * `dir_path` - Path to the directory to compress
/// * `output_path` - Path where the 7z file should be created
/// * `password` - Optional password to encrypt the archive
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

        // Extract and verify
        let extract_dir = temp_dir.path().join("extract");
        std::fs::create_dir(&extract_dir)?;

        let extract_dir_clone = extract_dir.clone();
        tokio::task::spawn_blocking(move || {
            sevenz_rust::decompress_file(output_path.as_path(), extract_dir_clone.as_path())
        })
        .await??;

        assert!(extract_dir.join("test1.txt").exists());
        assert!(extract_dir.join("test2.txt").exists());

        Ok(())
    }
}
