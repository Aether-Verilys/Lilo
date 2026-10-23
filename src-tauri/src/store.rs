use crate::model::{Field, NewTask, Task, TaskPatch};
use chrono::Utc;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::path::Path;

pub type Result<T> = std::result::Result<T, rusqlite::Error>;

/// 本地存储。所有读写都走这里，飞书同步引擎也只通过它落库，
/// 保证 SQLite 始终是唯一真相源。
pub struct Store {
    conn: Mutex<Connection>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS tasks (
    id           TEXT PRIMARY KEY,
    summary      TEXT NOT NULL,
    description  TEXT NOT NULL DEFAULT '',
    day          TEXT,
    remind_at    INTEGER,
    done         INTEGER NOT NULL DEFAULT 0,
    completed_at INTEGER,
    position     INTEGER NOT NULL DEFAULT 0,
    parent_id    TEXT REFERENCES tasks(id) ON DELETE CASCADE,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    lark_guid    TEXT UNIQUE,
    lark_url     TEXT,
    -- 飞书侧的 updated_at，用于比对新旧决定谁赢
    lark_updated_at INTEGER,
    conflict     INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_tasks_day    ON tasks(day);
CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_id);

-- 待推送到飞书的字段级脏标记。一条任务可有多行，各字段独立推送与消解。
CREATE TABLE IF NOT EXISTS dirty (
    task_id  TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    field    TEXT NOT NULL,
    since    INTEGER NOT NULL,
    PRIMARY KEY (task_id, field)
);

-- 本地删除的墓碑。没有它，飞书会把已删任务当新任务再同步回来。
CREATE TABLE IF NOT EXISTS tombstones (
    lark_guid  TEXT PRIMARY KEY,
    deleted_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    // ---------------- 读 ----------------

    /// 某一天的待办。`day` 传 None 取收集箱。只返回顶层任务，子待办另取。
    pub fn tasks_for_day(&self, day: Option<&str>) -> Result<Vec<Task>> {
        let conn = self.conn.lock();
        let sql = match day {
            Some(_) => {
                "SELECT * FROM tasks WHERE day = ?1 AND parent_id IS NULL \
                        ORDER BY done, position, created_at"
            }
            None => {
                "SELECT * FROM tasks WHERE day IS NULL AND parent_id IS NULL \
                     ORDER BY done, position, created_at"
            }
        };
        let mut stmt = conn.prepare(sql)?;
        let out: Vec<Task> = match day {
            Some(d) => stmt
                .query_map(params![d], row_to_task)?
                .collect::<Result<_>>()?,
            None => stmt.query_map([], row_to_task)?.collect::<Result<_>>()?,
        };
        Ok(out)
    }

    pub fn subtasks(&self, parent_id: &str) -> Result<Vec<Task>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT * FROM tasks WHERE parent_id = ?1 ORDER BY done, position, created_at",
        )?;
        let out: Vec<Task> = stmt
            .query_map(params![parent_id], row_to_task)?
            .collect::<Result<_>>()?;
        Ok(out)
    }

    /// 一段日期区间内每天的未完成/总数，供总览视图用。
    pub fn day_counts(&self, from: &str, to: &str) -> Result<Vec<(String, i64, i64)>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT day, SUM(CASE WHEN done = 0 THEN 1 ELSE 0 END), COUNT(*) \
             FROM tasks WHERE day BETWEEN ?1 AND ?2 AND parent_id IS NULL GROUP BY day",
        )?;
        let out: Vec<(String, i64, i64)> = stmt
            .query_map(params![from, to], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_>>()?;
        Ok(out)
    }

    pub fn get(&self, id: &str) -> Result<Option<Task>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT * FROM tasks WHERE id = ?1",
            params![id],
            row_to_task,
        )
        .optional()
    }

    pub fn by_lark_guid(&self, guid: &str) -> Result<Option<Task>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT * FROM tasks WHERE lark_guid = ?1",
            params![guid],
            row_to_task,
        )
        .optional()
    }

    // ---------------- 写 ----------------

    pub fn create(&self, new: &NewTask) -> Result<Task> {
        let now = Utc::now().timestamp_millis();
        let id = uuid::Uuid::new_v4().to_string();
        let conn = self.conn.lock();

        // 排到当天末尾
        let position: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(position), -1) + 1 FROM tasks \
                 WHERE day IS ?1 AND parent_id IS ?2",
                params![new.day, new.parent_id],
                |r| r.get(0),
            )
            .unwrap_or(0);

        conn.execute(
            "INSERT INTO tasks (id, summary, description, day, done, position, parent_id,
                                created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, ?7)",
            params![
                id,
                new.summary,
                new.description,
                new.day,
                position,
                new.parent_id,
                now
            ],
        )?;

        // 新任务整体待推送
        for f in [Field::Summary, Field::Description, Field::Day] {
            mark_dirty(&conn, &id, f, now)?;
        }

        conn.query_row(
            "SELECT * FROM tasks WHERE id = ?1",
            params![id],
            row_to_task,
        )
    }

    pub fn update(&self, id: &str, patch: &TaskPatch) -> Result<Option<Task>> {
        let now = Utc::now().timestamp_millis();
        let conn = self.conn.lock();

        if let Some(v) = &patch.summary {
            conn.execute(
                "UPDATE tasks SET summary = ?2 WHERE id = ?1",
                params![id, v],
            )?;
            mark_dirty(&conn, id, Field::Summary, now)?;
        }
        if let Some(v) = &patch.description {
            conn.execute(
                "UPDATE tasks SET description = ?2 WHERE id = ?1",
                params![id, v],
            )?;
            mark_dirty(&conn, id, Field::Description, now)?;
        }
        if let Some(v) = &patch.day {
            conn.execute("UPDATE tasks SET day = ?2 WHERE id = ?1", params![id, v])?;
            mark_dirty(&conn, id, Field::Day, now)?;
            // 飞书要求先有 due 才能有 reminder；挪进收集箱就必须清掉提醒
            if v.is_none() {
                conn.execute(
                    "UPDATE tasks SET remind_at = NULL WHERE id = ?1",
                    params![id],
                )?;
                mark_dirty(&conn, id, Field::Remind, now)?;
            }
        }
        if let Some(v) = &patch.remind_at {
            // 收集箱里的任务不接受提醒，静默忽略而不是写入一个飞书拒收的状态
            let has_day: bool = conn.query_row(
                "SELECT day IS NOT NULL FROM tasks WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )?;
            if has_day || v.is_none() {
                conn.execute(
                    "UPDATE tasks SET remind_at = ?2 WHERE id = ?1",
                    params![id, v],
                )?;
                mark_dirty(&conn, id, Field::Remind, now)?;
            }
        }
        if let Some(done) = patch.done {
            let completed_at = if done { Some(now) } else { None };
            conn.execute(
                "UPDATE tasks SET done = ?2, completed_at = ?3 WHERE id = ?1",
                params![id, done as i64, completed_at],
            )?;
            mark_dirty(&conn, id, Field::Done, now)?;
        }

        conn.execute(
            "UPDATE tasks SET updated_at = ?2 WHERE id = ?1",
            params![id, now],
        )?;
        conn.query_row(
            "SELECT * FROM tasks WHERE id = ?1",
            params![id],
            row_to_task,
        )
        .optional()
    }

    /// 删除任务。若已同步到飞书，留下墓碑防止被再次拉回。
    pub fn delete(&self, id: &str) -> Result<()> {
        let now = Utc::now().timestamp_millis();
        let conn = self.conn.lock();
        let guid: Option<String> = conn
            .query_row(
                "SELECT lark_guid FROM tasks WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        if let Some(g) = guid {
            conn.execute(
                "INSERT OR REPLACE INTO tombstones (lark_guid, deleted_at) VALUES (?1, ?2)",
                params![g, now],
            )?;
        }
        conn.execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn reorder(&self, day: Option<&str>, ordered_ids: &[String]) -> Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        for (i, id) in ordered_ids.iter().enumerate() {
            tx.execute(
                "UPDATE tasks SET position = ?2 WHERE id = ?1 AND day IS ?3",
                params![id, i as i64, day],
            )?;
        }
        tx.commit()
    }

    // ---------------- 同步引擎用 ----------------

    /// 取出待推送的脏字段，按任务聚合。
    pub fn pending_pushes(&self, limit: i64) -> Result<Vec<(Task, Vec<Field>)>> {
        let conn = self.conn.lock();
        let mut stmt =
            conn.prepare("SELECT DISTINCT task_id FROM dirty ORDER BY since LIMIT ?1")?;
        let ids: Vec<String> = stmt
            .query_map(params![limit], |r| r.get(0))?
            .collect::<Result<_>>()?;

        let mut out = Vec::new();
        for id in ids {
            let task = conn
                .query_row(
                    "SELECT * FROM tasks WHERE id = ?1",
                    params![&id],
                    row_to_task,
                )
                .optional()?;
            let Some(task) = task else { continue };
            let mut fs = conn.prepare("SELECT field FROM dirty WHERE task_id = ?1")?;
            let fields: Vec<Field> = fs
                .query_map(params![&id], |r| r.get::<_, String>(0))?
                .filter_map(|s| s.ok().and_then(|s| Field::parse(&s)))
                .collect();
            if !fields.is_empty() {
                out.push((task, fields));
            }
        }
        Ok(out)
    }

    pub fn pending_count(&self) -> Result<i64> {
        let conn = self.conn.lock();
        conn.query_row("SELECT COUNT(DISTINCT task_id) FROM dirty", [], |r| {
            r.get(0)
        })
    }

    /// 推送成功后清除脏标记并记录飞书返回的 updated_at。
    pub fn clear_dirty(
        &self,
        id: &str,
        fields: &[Field],
        lark_guid: Option<&str>,
        lark_url: Option<&str>,
        lark_updated_at: Option<i64>,
    ) -> Result<()> {
        let conn = self.conn.lock();
        for f in fields {
            conn.execute(
                "DELETE FROM dirty WHERE task_id = ?1 AND field = ?2",
                params![id, f.as_str()],
            )?;
        }
        if lark_guid.is_some() {
            conn.execute(
                "UPDATE tasks SET lark_guid = COALESCE(?2, lark_guid),
                        lark_url = COALESCE(?3, lark_url),
                        lark_updated_at = COALESCE(?4, lark_updated_at)
                 WHERE id = ?1",
                params![id, lark_guid, lark_url, lark_updated_at],
            )?;
        } else if lark_updated_at.is_some() {
            conn.execute(
                "UPDATE tasks SET lark_updated_at = ?2 WHERE id = ?1",
                params![id, lark_updated_at],
            )?;
        }
        Ok(())
    }

    pub fn is_tombstoned(&self, guid: &str) -> Result<bool> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM tombstones WHERE lark_guid = ?1)",
            params![guid],
            |r| r.get(0),
        )
    }

    /// 哪些字段仍是脏的 —— 拉取时跳过这些字段，避免覆盖用户尚未推送的本地改动。
    pub fn dirty_fields(&self, id: &str) -> Result<Vec<Field>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT field FROM dirty WHERE task_id = ?1")?;
        let out: Vec<Field> = stmt
            .query_map(params![id], |r| r.get::<_, String>(0))?
            .filter_map(|s| s.ok().and_then(|s| Field::parse(&s)))
            .collect();
        Ok(out)
    }

    pub fn set_conflict(&self, id: &str, on: bool) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE tasks SET conflict = ?2 WHERE id = ?1",
            params![id, on as i64],
        )?;
        Ok(())
    }

    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock();
        conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
            r.get(0)
        })
        .optional()
    }

    pub fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    /// 用飞书侧数据 upsert 一条任务。已存在时只写入非脏字段。
    /// 返回本地任务 id。
    pub fn apply_remote(&self, r: &RemoteTask) -> Result<Option<String>> {
        if self.is_tombstoned(&r.guid)? {
            return Ok(None);
        }
        let now = Utc::now().timestamp_millis();
        let existing = self.by_lark_guid(&r.guid)?;

        match existing {
            None => {
                let conn = self.conn.lock();
                let id = uuid::Uuid::new_v4().to_string();
                let position: i64 = conn
                    .query_row(
                        "SELECT COALESCE(MAX(position), -1) + 1 FROM tasks WHERE day IS ?1",
                        params![r.day],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
                conn.execute(
                    "INSERT INTO tasks (id, summary, description, day, remind_at, done,
                                        completed_at, position, created_at, updated_at,
                                        lark_guid, lark_url, lark_updated_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9,?10,?11,?12)",
                    params![
                        id,
                        r.summary,
                        r.description,
                        r.day,
                        r.remind_at,
                        r.done as i64,
                        r.completed_at,
                        position,
                        now,
                        r.guid,
                        r.url,
                        r.updated_at
                    ],
                )?;
                Ok(Some(id))
            }
            Some(local) => {
                let dirty = self.dirty_fields(&local.id)?;
                let seen: i64 = {
                    let conn = self.conn.lock();
                    conn.query_row(
                        "SELECT COALESCE(lark_updated_at, 0) FROM tasks WHERE id = ?1",
                        params![&local.id],
                        |row| row.get(0),
                    )?
                };
                // 飞书侧没有变新，本地不需要动
                if r.updated_at <= seen && dirty.is_empty() {
                    return Ok(Some(local.id));
                }
                let conn = self.conn.lock();
                let mut touched = false;
                // 两侧都改了同一字段：保留本地值，打上冲突标记让用户自己裁决
                let mut clash = false;

                if dirty.contains(&Field::Summary) {
                    clash = true;
                } else {
                    conn.execute(
                        "UPDATE tasks SET summary = ?2 WHERE id = ?1",
                        params![&local.id, &r.summary],
                    )?;
                    touched = true;
                }
                if dirty.contains(&Field::Description) {
                    clash = true;
                } else {
                    conn.execute(
                        "UPDATE tasks SET description = ?2 WHERE id = ?1",
                        params![&local.id, &r.description],
                    )?;
                    touched = true;
                }
                if dirty.contains(&Field::Day) {
                    clash = true;
                } else {
                    conn.execute(
                        "UPDATE tasks SET day = ?2 WHERE id = ?1",
                        params![&local.id, &r.day],
                    )?;
                    touched = true;
                }
                if dirty.contains(&Field::Remind) {
                    clash = true;
                } else {
                    conn.execute(
                        "UPDATE tasks SET remind_at = ?2 WHERE id = ?1",
                        params![&local.id, r.remind_at],
                    )?;
                    touched = true;
                }
                if dirty.contains(&Field::Done) {
                    clash = true;
                } else {
                    conn.execute(
                        "UPDATE tasks SET done = ?2, completed_at = ?3 WHERE id = ?1",
                        params![&local.id, r.done as i64, r.completed_at],
                    )?;
                    touched = true;
                }

                conn.execute(
                    "UPDATE tasks SET lark_updated_at = ?2, lark_url = COALESCE(?3, lark_url),
                            conflict = ?4, updated_at = CASE WHEN ?5 THEN ?6 ELSE updated_at END
                     WHERE id = ?1",
                    params![&local.id, r.updated_at, r.url, clash as i64, touched, now],
                )?;
                Ok(Some(local.id))
            }
        }
    }
}

/// 从飞书拉回来的、已归一化到本地语义的任务
#[derive(Debug, Clone)]
pub struct RemoteTask {
    pub guid: String,
    pub summary: String,
    pub description: String,
    pub day: Option<String>,
    pub remind_at: Option<i64>,
    pub done: bool,
    pub completed_at: Option<i64>,
    pub url: Option<String>,
    pub updated_at: i64,
}

fn mark_dirty(conn: &Connection, id: &str, f: Field, now: i64) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO dirty (task_id, field, since) VALUES (?1, ?2, ?3)",
        params![id, f.as_str(), now],
    )?;
    Ok(())
}

fn row_to_task(r: &Row) -> Result<Task> {
    Ok(Task {
        id: r.get("id")?,
        summary: r.get("summary")?,
        description: r.get("description")?,
        day: r.get("day")?,
        remind_at: r.get("remind_at")?,
        done: r.get::<_, i64>("done")? != 0,
        completed_at: r.get("completed_at")?,
        position: r.get("position")?,
        parent_id: r.get("parent_id")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
        lark_guid: r.get("lark_guid")?,
        lark_url: r.get("lark_url")?,
        conflict: r.get::<_, i64>("conflict")? != 0,
    })
}
