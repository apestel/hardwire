//use crossbeam::channel::{self, Sender};
use sqlx::{Pool, Sqlite};
use std::collections::HashMap;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, ReadBuf};
use tokio::sync::broadcast;

use serde::Serialize;

pub struct ProgressReader<R> {
    inner: R,
    total_bytes: u32,
    read_bytes: usize,
    transaction_id: String,
    file_path: String,
    ip_address: String,
    channel_sender: broadcast::Sender<Event>,
    start_offset: u64,
}

impl<R> ProgressReader<R> {
    pub fn new(
        inner: R,
        total_bytes: u32,
        transaction_id: String,
        file_path: String,
        ip_address: String,
        channel_sender: broadcast::Sender<Event>,
        start_offset: u64,
    ) -> Self {
        // CREATE TABLE downloads (
        // id INTEGER PRIMARY KEY AUTOINCREMENT,
        // ip_address TEXT,
        // transaction_id TEXT,
        // file_size INT,
        // started_at INT,
        // finished_at INT,
        // );

        Self {
            inner,
            total_bytes,
            read_bytes: 0,
            transaction_id,
            file_path,
            ip_address,
            channel_sender,
            start_offset,
        }
    }

    // pub fn progress(&self) -> f64 {
    //     (self.read_bytes as f64 / self.total_bytes as f64) * 100.0
    // }
}

impl<R: AsyncRead + Unpin> AsyncRead for ProgressReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let read_poll = Pin::new(&mut self.as_mut().inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(_)) = read_poll {
            self.read_bytes += buf.filled().len();
            self.channel_sender
                .send(Event::DownloadProgress(FileDownload {
                    file_path: self.file_path.clone(),
                    transaction_id: self.transaction_id.clone(),
                    ip_address: self.ip_address.clone(),
                    total_bytes: self.total_bytes,
                    read_bytes: self.read_bytes,
                    start_offset: self.start_offset,
                }))
                .unwrap();
        }
        read_poll
    }
}
#[derive(Debug, Clone, Copy)]
pub enum DownloadStatus {
    Complete,
}

impl DownloadStatus {
    pub fn to_str(self) -> String {
        match self {
            DownloadStatus::Complete => "complete".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FileDownload {
    total_bytes: u32,
    read_bytes: usize,
    transaction_id: String,
    file_path: String,
    ip_address: String,
    start_offset: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event")]
#[serde(rename_all = "snake_case")]
pub enum Event {
    DownloadProgress(FileDownload),
}
#[derive(Debug, Clone)]
pub struct Manager {
    pub sender: broadcast::Sender<Event>,
    db_pool: Pool<Sqlite>,
    ongoing_download: HashMap<String, FileDownload>,
}

impl Manager {
    pub fn new(db_pool: Pool<Sqlite>) -> Self {
        let (send, _) = broadcast::channel::<Event>(6000);
        Manager {
            sender: send,
            db_pool,
            ongoing_download: HashMap::new(),
        }
    }

    pub async fn start_recv_thread(&mut self) {
        let mut mgr = self.clone();
        tokio::spawn(async move { mgr.process_message().await });
    }

    async fn process_message(&mut self) {
        let mut receiver = self.sender.subscribe();
        loop {
            let m = receiver.recv().await;
            match m {
                Ok(m) => match m {
                    Event::DownloadProgress(pm) => {
                        self.update_download_progress(pm).await;
                    }
                },
                Err(err) => tracing::error!("Progress queue receiver have been ended: {}", err),
            }
        }
    }

    async fn update_download_progress(&mut self, pm: FileDownload) {
        let transaction_id = pm.transaction_id.clone();
        let now = chrono::Utc::now().timestamp();

        if !self.ongoing_download.contains_key(&transaction_id) {
            // First chunk: insert the row with started_at
            sqlx::query!(
                "INSERT INTO download (file_path, ip_address, transaction_id, status, file_size, started_at) VALUES ($1, $2, $3, 'in_progress', $4, $5)",
                pm.file_path,
                pm.ip_address,
                pm.transaction_id,
                pm.total_bytes,
                now,
            )
            .execute(&self.db_pool)
            .await
            .unwrap();
        }

        if pm.total_bytes > 0 && pm.read_bytes as u32 >= pm.total_bytes {
            // Last chunk: mark as complete with finished_at
            let status = DownloadStatus::Complete.to_str();
            sqlx::query!(
                "UPDATE download SET status = $1, finished_at = $2 WHERE transaction_id = $3",
                status,
                now,
                pm.transaction_id,
            )
            .execute(&self.db_pool)
            .await
            .unwrap();
            self.ongoing_download.remove(&transaction_id);
        } else {
            self.ongoing_download.insert(transaction_id, pm);
        }
    }
}
