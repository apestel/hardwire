pub mod tasks;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", content = "data")]
pub enum TaskInput {
    CreateArchive(ArchiveInput),
    // Add other task types here
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ArchiveInput {
    pub files: Option<Vec<PathBuf>>,
    pub directory: Option<PathBuf>,
    pub password: Option<String>,
    pub output_path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::Type)]
#[sqlx(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskStatus::Pending => write!(f, "pending"),
            TaskStatus::Running => write!(f, "running"),
            TaskStatus::Completed => write!(f, "completed"),
            TaskStatus::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Task {
    pub id: String,
    pub status: TaskStatus,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub error: Option<String>,
    pub progress: i32,
}

#[derive(Debug, Clone)]
pub struct TaskManager {
    pub(crate) db: SqlitePool,
    _task_sender: mpsc::Sender<String>, // Task ID
}

impl TaskManager {
    pub fn new(db: SqlitePool) -> (Self, mpsc::Receiver<String>) {
        let (tx, rx) = mpsc::channel(32);
        (
            Self {
                db,
                _task_sender: tx,
            },
            rx,
        )
    }

    pub async fn create_task(&self, input: TaskInput) -> Result<String> {
        let task_id = Uuid::new_v4().to_string();
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;

        let input_str = serde_json::to_string(&input)?;
        let task_type = format!("{:?}", input);
        let task_status = TaskStatus::Pending.to_string();

        sqlx::query!(
            r#"
            INSERT INTO tasks (id, task_type, status, created_at, input_data, progress)
            VALUES (?, ?, ?, ?, ?, 0)
            "#,
            task_id,
            task_type,
            task_status,
            now,
            input_str,
        )
        .execute(&self.db)
        .await?;

        // Send task to worker
        self._task_sender.send(task_id.clone()).await?;

        Ok(task_id)
    }

    pub async fn get_task_status(&self, task_id: &str) -> Result<Task> {
        let task = sqlx::query!(
            r#"
            SELECT 
                id,
                status as "status: TaskStatus",
                created_at,
                started_at,
                finished_at,
                error,
                COALESCE(progress, 0) as "progress!: i32"
            FROM tasks
            WHERE id = ?
            "#,
            task_id
        )
        .fetch_one(&self.db)
        .await?;

        Ok(Task {
            id: task.id,
            status: task.status,
            created_at: task.created_at,
            started_at: task.started_at,
            finished_at: task.finished_at,
            error: task.error,
            progress: task.progress,
        })
    }

    pub async fn update_task_status(
        &self,
        task_id: &str,
        status: TaskStatus,
        error: Option<String>,
        progress: Option<i32>,
    ) -> Result<()> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;

        let mut query = String::from("UPDATE tasks SET status = ?, error = COALESCE(?, error)");

        let mut values: Vec<String> = vec![status.to_string(), error.unwrap_or_default()];

        if let Some(prog) = progress {
            query.push_str(", progress = ?");
            values.push(prog.to_string());
        }

        match status {
            TaskStatus::Running => {
                query.push_str(", started_at = ?");
                values.push(now.to_string());
            }
            TaskStatus::Completed | TaskStatus::Failed => {
                query.push_str(", finished_at = ?");
                values.push(now.to_string());
            }
            _ => {}
        }

        query.push_str(" WHERE id = ?");
        values.push(task_id.to_string());

        let mut q = sqlx::query(&query);
        for value in values {
            q = q.bind(value);
        }

        q.execute(&self.db).await?;

        Ok(())
    }
}
