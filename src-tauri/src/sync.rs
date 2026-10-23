use crate::lark::{Lark, LarkError};
use crate::model::SyncStatus;
use crate::store::Store;
use chrono::Utc;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// 飞书双向同步引擎。
///
/// 两条腿，都不在后台空转：
/// 1. 推送 —— 本地脏字段防抖后批量提交
/// 2. 全量对账 —— 打开/聚焦面板时拉一次（有冷却，避免连点）
pub struct Sync {
    store: Arc<Store>,
    lark: Arc<Lark>,
    app: AppHandle,
    state: Mutex<SyncState>,
    enabled: AtomicBool,
    /// 防止全量对账与推送同时跑
    running: Mutex<()>,
    last_pull: Mutex<Option<Instant>>,
}

struct SyncState {
    label: String,
    last_sync_at: Option<i64>,
    message: Option<String>,
}

impl Sync {
    pub fn new(store: Arc<Store>, lark: Arc<Lark>, app: AppHandle, enabled: bool) -> Arc<Self> {
        Arc::new(Self {
            store,
            lark,
            app,
            state: Mutex::new(SyncState {
                label: if enabled { "idle" } else { "disabled" }.to_string(),
                last_sync_at: None,
                message: None,
            }),
            enabled: AtomicBool::new(enabled),
            running: Mutex::new(()),
            last_pull: Mutex::new(None),
        })
    }

    pub fn status(&self) -> SyncStatus {
        let s = self.state.lock();
        SyncStatus {
            state: s.label.clone(),
            last_sync_at: s.last_sync_at,
            pending: self.store.pending_count().unwrap_or(0),
            message: s.message.clone(),
        }
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::SeqCst);
        self.set_state(if on { "idle" } else { "disabled" }, None);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    fn set_state(&self, label: &str, message: Option<String>) {
        {
            let mut s = self.state.lock();
            s.label = label.to_string();
            s.message = message;
        }
        let _ = self.app.emit("sync-status", self.status());
    }

    fn mark_synced(&self) {
        {
            let mut s = self.state.lock();
            s.last_sync_at = Some(Utc::now().timestamp_millis());
            s.label = "idle".to_string();
            s.message = None;
        }
        let _ = self.app.emit("sync-status", self.status());
    }

    /// 把 LarkError 映射成状态栏能显示的状态，并决定是否值得重试
    fn report(&self, e: &LarkError) {
        let label = match e {
            LarkError::Offline => "offline",
            LarkError::Unauthorized | LarkError::MissingScope(_) => "unauthorized",
            LarkError::NotInstalled => "disabled",
            _ => "error",
        };
        self.set_state(label, Some(e.to_string()));
    }

    // ---------------- 推送 ----------------

    /// 提交所有脏字段。返回成功推送的任务数。
    pub fn push_once(&self) -> usize {
        if !self.is_enabled() {
            return 0;
        }
        let _guard = self.running.lock();
        let batch = match self.store.pending_pushes(50) {
            Ok(b) => b,
            Err(e) => {
                self.set_state("error", Some(e.to_string()));
                return 0;
            }
        };
        if batch.is_empty() {
            return 0;
        }
        self.set_state("syncing", None);

        let mut ok = 0usize;
        for (task, fields) in batch {
            let res = match &task.lark_guid {
                // 已有 guid：增量 patch
                Some(guid) => self
                    .lark
                    .patch_task(
                        guid,
                        &fields,
                        &task.summary,
                        &task.description,
                        task.day.as_deref(),
                        task.remind_at,
                        task.done,
                        task.completed_at,
                    )
                    .map(|updated| (None, None, Some(updated))),
                // 还没同步过：新建。local_id 当 client_token，重试幂等。
                None => self
                    .lark
                    .create_task(
                        &task.id,
                        &task.summary,
                        &task.description,
                        task.day.as_deref(),
                        task.remind_at,
                    )
                    .map(|(guid, url, updated)| (Some(guid), url, Some(updated))),
            };

            match res {
                Ok((guid, url, updated)) => {
                    if let Err(e) = self.store.clear_dirty(
                        &task.id,
                        &fields,
                        guid.as_deref(),
                        url.as_deref(),
                        updated,
                    ) {
                        self.set_state("error", Some(e.to_string()));
                        break;
                    }
                    ok += 1;
                }
                Err(e) => {
                    // 网络/授权类错误无需继续试后面的，留着下次推
                    let fatal = matches!(
                        e,
                        LarkError::Offline
                            | LarkError::Unauthorized
                            | LarkError::MissingScope(_)
                            | LarkError::NotInstalled
                    );
                    self.report(&e);
                    if fatal {
                        return ok;
                    }
                }
            }
        }
        if ok > 0 {
            self.mark_synced();
            let _ = self.app.emit("tasks-changed", ());
        }
        ok
    }

    // ---------------- 全量对账 ----------------

    /// 拉全量并 upsert。手动「同步」按钮走这里，不受冷却限制。
    pub fn reconcile(&self) -> usize {
        if !self.is_enabled() {
            return 0;
        }
        let _guard = self.running.lock();
        self.reconcile_inner()
    }

    /// 打开面板时拉一次。冷却内重复触发直接跳过，避免聚焦抖动打爆 API。
    pub fn pull_if_stale(&self, min_secs: u64) -> usize {
        if !self.is_enabled() {
            return 0;
        }
        let _guard = self.running.lock();
        if let Some(t) = *self.last_pull.lock() {
            if t.elapsed() < Duration::from_secs(min_secs) {
                return 0;
            }
        }
        self.reconcile_inner()
    }

    fn reconcile_inner(&self) -> usize {
        self.set_state("syncing", None);

        let remote = match self.lark.list_my_tasks() {
            Ok(r) => r,
            Err(e) => {
                self.report(&e);
                return 0;
            }
        };
        let mut n = 0usize;
        for r in &remote {
            match self.store.apply_remote(r) {
                Ok(Some(_)) => n += 1,
                Ok(None) => {} // 墓碑命中，已删的不再拉回
                Err(e) => {
                    self.set_state("error", Some(e.to_string()));
                    return n;
                }
            }
        }
        *self.last_pull.lock() = Some(Instant::now());
        self.mark_synced();
        let _ = self.app.emit("tasks-changed", ());
        n
    }

    /// 防抖推送线程：每 2 秒看一次有没有脏字段，有就推。
    /// 比「每次编辑都立刻推」省得多，连续打字不会打爆 API。
    pub fn spawn_push_loop(self: &Arc<Self>) {
        let this = Arc::clone(self);
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(2));
            if this.is_enabled() && this.store.pending_count().unwrap_or(0) > 0 {
                this.push_once();
            }
        });
    }
}
