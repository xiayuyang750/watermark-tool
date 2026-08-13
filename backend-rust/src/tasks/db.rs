//! 任务持久化（SQLite），逻辑与 Python 版 backend/app/tasks/db.py 一致。

use rusqlite::{params, Connection};
use std::path::PathBuf;

use crate::config::data_dir;

#[derive(Debug, Clone)]
pub struct TaskRow {
    pub id: String,
    pub task_type: String,
    pub status: String,
    pub progress: i32,
    pub output: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
}

pub struct DB {
    path: PathBuf,
}

impl DB {
    pub fn new() -> Self {
        let db = DB {
            path: data_dir().join("tasks.db"),
        };
        db.init();
        db
    }

    fn conn(&self) -> rusqlite::Result<Connection> {
        Connection::open(&self.path)
    }

    fn init(&self) {
        let conn = self.conn().expect("打开任务数据库失败");
        conn.execute(
            "CREATE TABLE IF NOT EXISTS tasks(
               id TEXT PRIMARY KEY, type TEXT, status TEXT, progress INTEGER,
               output TEXT, error TEXT, created_at TEXT, options TEXT)",
            [],
        )
        .expect("初始化任务表失败");
    }

    pub fn upsert(
        &self,
        tid: &str,
        task_type: &str,
        status: &str,
        progress: i32,
        output: Option<&str>,
        error: Option<&str>,
        created_at: &str,
        options: &serde_json::Value,
    ) -> rusqlite::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO tasks(id, type, status, progress, output, error, created_at, options)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(id) DO UPDATE SET
               type=excluded.type, status=excluded.status, progress=excluded.progress,
               output=excluded.output, error=excluded.error, options=excluded.options",
            params![
                tid,
                task_type,
                status,
                progress,
                output,
                error,
                created_at,
                serde_json::to_string(options).unwrap_or_else(|_| "{}".to_string()),
            ],
        )?;
        Ok(())
    }

    pub fn update(
        &self,
        tid: &str,
        status: Option<&str>,
        progress: Option<i32>,
        output: Option<&str>,
        error: Option<&str>,
    ) -> rusqlite::Result<()> {
        let mut sets: Vec<String> = Vec::new();
        let mut values: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(v) = status {
            sets.push("status=?".into());
            values.push(rusqlite::types::Value::Text(v.to_string()));
        }
        if let Some(v) = progress {
            sets.push("progress=?".into());
            values.push(rusqlite::types::Value::Integer(v.into()));
        }
        if let Some(v) = output {
            sets.push("output=?".into());
            values.push(rusqlite::types::Value::Text(v.to_string()));
        }
        if let Some(v) = error {
            sets.push("error=?".into());
            values.push(rusqlite::types::Value::Text(v.to_string()));
        }
        if sets.is_empty() {
            return Ok(());
        }
        let sql = format!("UPDATE tasks SET {} WHERE id=?", sets.join(", "));
        let mut params: Vec<rusqlite::types::Value> = values;
        params.push(rusqlite::types::Value::Text(tid.to_string()));
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&sql)?;
        stmt.execute(rusqlite::params_from_iter(params.iter()))?;
        Ok(())
    }

    /// 与 Python 版 db.py 的 get 对应（manager 主要用内存态，保留作持久化查询入口）。
    #[allow(dead_code)]
    pub fn get(&self, tid: &str) -> rusqlite::Result<Option<TaskRow>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, type, status, progress, output, error, created_at FROM tasks WHERE id=?",
        )?;
        let mut rows = stmt.query_map([tid], row_to_task)?;
        Ok(rows.next().transpose()?)
    }

    pub fn list_all(&self) -> rusqlite::Result<Vec<TaskRow>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, type, status, progress, output, error, created_at FROM tasks ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], row_to_task)?;
        rows.collect()
    }
}

fn row_to_task(row: &rusqlite::Row) -> rusqlite::Result<TaskRow> {
    Ok(TaskRow {
        id: row.get(0)?,
        task_type: row.get(1)?,
        status: row.get(2)?,
        progress: row.get(3)?,
        output: row.get(4)?,
        error: row.get(5)?,
        created_at: row.get(6)?,
    })
}
