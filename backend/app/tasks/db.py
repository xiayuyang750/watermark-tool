"""任务持久化（SQLite）。"""
import sqlite3
import json

from ..config import APP_DIR


class DB:
    def __init__(self):
        self.path = APP_DIR / "tasks.db"
        self._init()

    def _conn(self) -> sqlite3.Connection:
        return sqlite3.connect(self.path)

    def _init(self) -> None:
        with self._conn() as c:
            c.execute(
                "CREATE TABLE IF NOT EXISTS tasks("
                "id TEXT PRIMARY KEY, type TEXT, status TEXT, progress INTEGER, "
                "output TEXT, error TEXT, created_at TEXT, options TEXT)"
            )

    def upsert(self, tid: str, **fields) -> None:
        with self._conn() as c:
            c.execute(
                "INSERT INTO tasks(id, type, status, progress, output, error, created_at, options) "
                "VALUES(?,?,?,?,?,?,?,?) "
                "ON CONFLICT(id) DO UPDATE SET "
                "type=excluded.type, status=excluded.status, progress=excluded.progress, "
                "output=excluded.output, error=excluded.error, options=excluded.options",
                (
                    tid,
                    fields.get("type", "link"),
                    fields.get("status", "pending"),
                    fields.get("progress", 0),
                    fields.get("output"),
                    fields.get("error"),
                    fields.get("created_at", ""),
                    json.dumps(fields.get("options", {}), ensure_ascii=False),
                ),
            )

    def update(self, tid: str, **fields) -> None:
        cols = {k: v for k, v in fields.items() if k in ("status", "progress", "output", "error")}
        if not cols:
            return
        sets = ", ".join(f"{k}=?" for k in cols)
        with self._conn() as c:
            c.execute(f"UPDATE tasks SET {sets} WHERE id=?", (*cols.values(), tid))

    def get(self, tid: str) -> dict | None:
        with self._conn() as c:
            row = c.execute(
                "SELECT id, type, status, progress, output, error, created_at FROM tasks WHERE id=?",
                (tid,),
            ).fetchone()
        if not row:
            return None
        return {
            "id": row[0], "type": row[1], "status": row[2], "progress": row[3],
            "output": row[4], "error": row[5], "created_at": row[6],
        }

    def list_all(self) -> list[dict]:
        with self._conn() as c:
            rows = c.execute(
                "SELECT id, type, status, progress, output, error, created_at "
                "FROM tasks ORDER BY created_at DESC"
            ).fetchall()
        return [
            {
                "id": r[0], "type": r[1], "status": r[2], "progress": r[3],
                "output": r[4], "error": r[5], "created_at": r[6],
            }
            for r in rows
        ]
