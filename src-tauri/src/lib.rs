mod lark;
mod model;
mod snap;
mod store;
mod sync;

/// 集成测试用的内部接口。同步引擎的映射与冲突规则是最容易出错的地方，
/// 值得对着真实响应结构做断言。
pub mod testing {
    pub use crate::lark::{
        is_already_completed_reject, omit_completed_at_patch, parse_login_pending_for_test,
        parse_remote_for_test, Lark,
    };
    pub use crate::model::{Field, NewTask, TaskPatch};
    pub use crate::store::{RemoteTask, Store};
}

use lark::Lark;
use model::{NewTask, SyncStatus, Task, TaskPatch};
use std::collections::HashMap;
use std::sync::Arc;
use store::Store;
use sync::Sync;
use tauri::{Emitter, Manager};

/// 吸附时锁定的显示器信息。
///
/// 折叠后窗口大部分在屏幕外，此时 `current_monitor()` 会返回相邻的那块屏，
/// 再按它算坐标就会让窗口一路走到别的显示器上去。所以吸附那一刻就把
/// 显示器范围记下来，折叠/展开全程复用，不再重新查询。
#[derive(Debug, Clone, Copy)]
struct SnapAnchor {
    side: snap::Side,
    bounds: snap::Bounds,
    scale: f64,
    /// 是否处于折叠态。折叠时窗口大部分在屏幕外，任何窗口移动事件都不能
    /// 触发重新判定 —— 否则会按「窗口当前压在哪块屏」重算，把锚点改到
    /// 隔壁显示器，窗口就一路漂过去了。
    collapsed: bool,
}

struct AppState {
    store: Arc<Store>,
    lark: Arc<Lark>,
    sync: Arc<Sync>,
    anchors: parking_lot::Mutex<HashMap<String, SnapAnchor>>,
}

/// 打开本地库。优先 `lilo.db`；没有的话从旧版 `kite.db` 迁过来，
/// 避免改名后用户数据凭空消失。
fn resolve_db_path(dir: &std::path::Path) -> std::path::PathBuf {
    let db = dir.join("lilo.db");
    if db.exists() {
        return db;
    }
    let here = dir.join("kite.db");
    if here.exists() {
        let _ = copy_sqlite(&here, &db);
        return db;
    }
    if let Some(old) = find_legacy_kite_db() {
        let _ = copy_sqlite(&old, &db);
    }
    db
}

fn find_legacy_kite_db() -> Option<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(roaming) = std::env::var("APPDATA") {
        let r = std::path::PathBuf::from(roaming);
        dirs.push(r.join("cn.kitlib.kite"));
        dirs.push(r.join("com.kitlib.kite"));
        dirs.push(r.join("Kite"));
        dirs.push(r.join("kite"));
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let l = std::path::PathBuf::from(local);
        dirs.push(l.join("cn.kitlib.kite"));
        dirs.push(l.join("com.kitlib.kite"));
        dirs.push(l.join("Kite"));
        dirs.push(l.join("kite"));
    }
    dirs.into_iter()
        .map(|d| d.join("kite.db"))
        .find(|p| p.exists())
}

fn copy_sqlite(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(from, to)?;
    for suffix in ["-wal", "-shm"] {
        let mut src = from.as_os_str().to_os_string();
        src.push(suffix);
        let src = std::path::PathBuf::from(src);
        if src.exists() {
            let mut dst = to.as_os_str().to_os_string();
            dst.push(suffix);
            let _ = std::fs::copy(&src, std::path::PathBuf::from(dst));
        }
    }
    Ok(())
}

/// 前端拿到的错误一律是字符串，避免把内部类型泄到 UI 层
type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

// ---------------- 待办读写 ----------------

#[tauri::command]
fn list_tasks(state: tauri::State<AppState>, day: Option<String>) -> CmdResult<Vec<Task>> {
    state.store.tasks_for_day(day.as_deref()).map_err(err)
}

#[tauri::command]
fn list_subtasks(state: tauri::State<AppState>, parent_id: String) -> CmdResult<Vec<Task>> {
    state.store.subtasks(&parent_id).map_err(err)
}

#[tauri::command]
fn day_counts(
    state: tauri::State<AppState>,
    from: String,
    to: String,
) -> CmdResult<Vec<(String, i64, i64)>> {
    state.store.day_counts(&from, &to).map_err(err)
}

#[tauri::command]
fn create_task(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    new: NewTask,
) -> CmdResult<Task> {
    let t = state.store.create(&new).map_err(err)?;
    let _ = app.emit("tasks-changed", ());
    Ok(t)
}

#[tauri::command]
fn update_task(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
    patch: TaskPatch,
) -> CmdResult<Option<Task>> {
    let t = state.store.update(&id, &patch).map_err(err)?;
    let _ = app.emit("tasks-changed", ());
    Ok(t)
}

#[tauri::command]
fn delete_task(app: tauri::AppHandle, state: tauri::State<AppState>, id: String) -> CmdResult<()> {
    state.store.delete(&id).map_err(err)?;
    let _ = app.emit("tasks-changed", ());
    Ok(())
}

#[tauri::command]
fn reorder_tasks(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    day: Option<String>,
    ids: Vec<String>,
) -> CmdResult<()> {
    state.store.reorder(day.as_deref(), &ids).map_err(err)?;
    let _ = app.emit("tasks-changed", ());
    Ok(())
}

/// 清掉冲突标记 —— 用户看过并接受了本地版本
#[tauri::command]
fn resolve_conflict(state: tauri::State<AppState>, id: String) -> CmdResult<()> {
    state.store.set_conflict(&id, false).map_err(err)
}

// ---------------- 同步 ----------------

#[tauri::command]
fn sync_status(state: tauri::State<AppState>) -> SyncStatus {
    state.sync.status()
}

#[tauri::command]
fn sync_now(state: tauri::State<AppState>) -> CmdResult<usize> {
    let pushed = state.sync.push_once();
    let pulled = state.sync.reconcile();
    Ok(pushed + pulled)
}

/// 打开/聚焦面板时拉一次飞书。冷却内重复调用直接跳过。
#[tauri::command]
fn pull_on_view(state: tauri::State<AppState>) -> CmdResult<usize> {
    Ok(state.sync.pull_if_stale(20))
}

#[tauri::command]
fn set_sync_enabled(state: tauri::State<AppState>, on: bool) -> CmdResult<()> {
    state.sync.set_enabled(on);
    state
        .store
        .meta_set("sync_enabled", if on { "1" } else { "0" })
        .map_err(err)
}

/// 检查飞书授权情况。返回缺失的 scope 列表，空表示齐了。
#[tauri::command]
fn lark_check(state: tauri::State<AppState>) -> CmdResult<Vec<String>> {
    state.lark.missing_write_scopes().map_err(err)
}

#[tauri::command]
fn lark_account(state: tauri::State<AppState>) -> CmdResult<lark::LarkAccount> {
    state.lark.account().map_err(err)
}

/// 一键补飞书写权限：打开浏览器扫码页，等用户点完同意后返回仍缺失的 scope。
/// 扫码可能要等几分钟，放到阻塞线程，避免卡住 UI 事件循环。
#[tauri::command]
async fn lark_grant_write(state: tauri::State<'_, AppState>) -> CmdResult<Vec<String>> {
    let lark = Arc::clone(&state.lark);
    let sync = Arc::clone(&state.sync);
    tauri::async_runtime::spawn_blocking(move || {
        let missing = lark.missing_write_scopes().map_err(err)?;
        if missing.is_empty() {
            return Ok(missing);
        }
        let pending = lark.start_login(&missing).map_err(err)?;
        tauri_plugin_opener::open_url(&pending.verification_url, None::<&str>)
            .map_err(|e| format!("无法打开授权页：{e}"))?;
        lark.complete_login(&pending.device_code).map_err(err)?;
        let still = lark.missing_write_scopes().map_err(err)?;
        if still.is_empty() {
            let _ = sync.reconcile();
        }
        Ok(still)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn snap_debug(msg: String) {
    snap_log(&format!("[ui] {msg}"));
}

/// 让前端把调试信息写进同一份 snap 日志，便于排查 WebView 里发生了什么。
fn snap_log(msg: &str) {
    if std::env::var("LILO_SNAP_DEBUG")
        .or_else(|_| std::env::var("KITE_SNAP_DEBUG"))
        .is_err()
    {
        return;
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(std::env::temp_dir().join("lilo-snap.log"))
    {
        let _ = writeln!(f, "{msg}");
    }
}

/// 拖动结束后判断是否该吸附。返回吸附到哪边（null 表示没吸附）。
#[tauri::command]
fn snap_check(
    w: tauri::WebviewWindow,
    state: tauri::State<AppState>,
) -> CmdResult<Option<snap::Side>> {
    let label = w.label().to_string();
    let is_mini = label == "mini";

    // 折叠态下窗口位置是我们自己摆的，不是用户拖的；重新判定只会把锚点
    // 算到隔壁屏幕上去。直接沿用已有锚点。
    {
        if let Some(a) = state.anchors.lock().get(&label).copied() {
            if a.collapsed {
                snap_log("snap_check: 折叠中，跳过重新判定");
                return Ok(Some(a.side));
            }
        }
    }
    let pos = w.outer_position().map_err(err)?;
    let size = w.outer_size().map_err(err)?;
    let mon = w
        .current_monitor()
        .map_err(err)?
        .ok_or_else(|| "找不到当前显示器".to_string())?;
    let scale = mon.scale_factor();
    let bounds = snap::Bounds {
        x: mon.position().x,
        width: mon.size().width as i32,
    };

    let side = snap::compute_snap(bounds, pos.x, size.width as i32, scale);
    snap_log(&format!(
        "snap_check: win=({},{}) size={}x{} mon.x={} mon.w={} scale={} -> {:?}",
        pos.x, pos.y, size.width, size.height, bounds.x, bounds.width, scale, side
    ));

    // 记下这一刻的显示器，折叠期间不再重新查询
    {
        let mut anchors = state.anchors.lock();
        if let Some(s) = side {
            anchors.insert(
                label.clone(),
                SnapAnchor {
                    side: s,
                    bounds,
                    scale,
                    collapsed: false,
                },
            );
        } else {
            anchors.remove(&label);
        }
    }

    let meta_key = if is_mini { "mini_snap" } else { "main_snap" };

    match side {
        Some(s) => {
            // 贴平到边缘，y 夹进屏幕内
            let x = snap::expanded_x(bounds, s, size.width as i32);
            let y = snap::clamp_y(
                mon.position().y,
                mon.size().height as i32,
                pos.y,
                size.height as i32,
            );
            w.set_position(tauri::PhysicalPosition::new(x, y))
                .map_err(err)?;
            let _ = state.store.meta_set(
                meta_key,
                match s {
                    snap::Side::Left => "left",
                    snap::Side::Right => "right",
                },
            );
        }
        None => {
            let _ = state.store.meta_set(meta_key, "");
        }
    }
    Ok(side)
}

/// 不依赖拖动位置，直接把迷你浮窗吸附到当前显示器指定的一侧。
/// 既作为明确可见的操作入口，也能在部分窗口管理器不稳定发送 moved 事件时兜底。
#[tauri::command]
fn snap_to(
    w: tauri::WebviewWindow,
    state: tauri::State<AppState>,
    side: snap::Side,
) -> CmdResult<()> {
    let is_mini = w.label() == "mini";
    let label = w.label().to_string();
    let pos = w.outer_position().map_err(err)?;
    let size = w.outer_size().map_err(err)?;
    let mon = w
        .current_monitor()
        .map_err(err)?
        .ok_or_else(|| "找不到当前显示器".to_string())?;
    let bounds = snap::Bounds {
        x: mon.position().x,
        width: mon.size().width as i32,
    };
    let anchor = SnapAnchor {
        side,
        bounds,
        scale: mon.scale_factor(),
        collapsed: false,
    };
    state.anchors.lock().insert(label, anchor);

    let x = snap::expanded_x(bounds, side, size.width as i32);
    let y = snap::clamp_y(
        mon.position().y,
        mon.size().height as i32,
        pos.y,
        size.height as i32,
    );
    w.set_position(tauri::PhysicalPosition::new(x, y))
        .map_err(err)?;
    state
        .store
        .meta_set(
            if is_mini { "mini_snap" } else { "main_snap" },
            match side {
                snap::Side::Left => "left",
                snap::Side::Right => "right",
            },
        )
        .map_err(err)?;
    snap_log(&format!("snap_to: side={side:?} -> ({x},{y})"));
    Ok(())
}

/// 取消吸附并清除持久化状态，避免应用重启后又恢复到侧边。
#[tauri::command]
fn snap_unsnap(w: tauri::WebviewWindow, state: tauri::State<AppState>) -> CmdResult<()> {
    state.anchors.lock().remove(w.label());
    let key = if w.label() == "mini" {
        "mini_snap"
    } else {
        "main_snap"
    };
    state.store.meta_set(key, "").map_err(err)
}

/// 取吸附时锁定的显示器；没有锚点（例如刚重启）就用当前显示器补一个。
fn anchor_for(
    w: &tauri::WebviewWindow,
    state: &tauri::State<AppState>,
    side: snap::Side,
) -> CmdResult<SnapAnchor> {
    if let Some(a) = state.anchors.lock().get(w.label()).copied() {
        if a.side == side {
            return Ok(a);
        }
    }
    let mon = w
        .current_monitor()
        .map_err(err)?
        .ok_or_else(|| "找不到当前显示器".to_string())?;
    let a = SnapAnchor {
        side,
        bounds: snap::Bounds {
            x: mon.position().x,
            width: mon.size().width as i32,
        },
        scale: mon.scale_factor(),
        collapsed: false,
    };
    state.anchors.lock().insert(w.label().to_string(), a);
    Ok(a)
}

/// 光标是否还在窗口外框上（含标题栏和缩放边）。
/// WebView 的 mouseleave 在鼠标移到标题栏时就会触发，不能用来判断「离开面板」。
#[tauri::command]
fn snap_pointer_over(w: tauri::WebviewWindow) -> CmdResult<bool> {
    let pos = w.outer_position().map_err(err)?;
    let size = w.outer_size().map_err(err)?;
    let cursor = w.cursor_position().map_err(err)?;
    // Windows 缩放热区会伸出窗口几个像素
    const PAD: i32 = 12;
    let x = cursor.x.round() as i32;
    let y = cursor.y.round() as i32;
    let hit = x >= pos.x - PAD
        && x < pos.x + size.width as i32 + PAD
        && y >= pos.y - PAD
        && y < pos.y + size.height as i32 + PAD;
    snap_log(&format!(
        "pointer_over: cursor=({x},{y}) win=({},{}) {}x{} -> {hit}",
        pos.x, pos.y, size.width, size.height
    ));
    Ok(hit)
}

/// 在工作线程上移动原生窗口，避免阻塞 WebView 和系统窗口消息。
async fn slide_window(w: tauri::WebviewWindow, x: i32) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let start = w.outer_position().map_err(err)?;
        if start.x == x {
            return Ok(());
        }
        let began = std::time::Instant::now();
        let duration = std::time::Duration::from_millis(160);
        loop {
            let progress = (began.elapsed().as_secs_f64() / duration.as_secs_f64()).min(1.0);
            let eased = 1.0 - (1.0 - progress).powi(3);
            let next_x = (start.x as f64 + (x as f64 - start.x as f64) * eased).round() as i32;
            w.set_position(tauri::PhysicalPosition::new(next_x, start.y))
                .map_err(err)?;
            if progress >= 1.0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
        Ok(())
    })
    .await
    .map_err(err)?
}

/// 折叠：滑出屏幕只留一条窄边。
#[tauri::command]
async fn snap_collapse(
    w: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    side: snap::Side,
) -> CmdResult<()> {
    let size = w.outer_size().map_err(err)?;
    let a = anchor_for(&w, &state, side)?;
    let x = snap::collapsed_x(a.bounds, side, size.width as i32, a.scale);
    snap_log(&format!(
        "collapse: side={side:?} -> x={x} (mon.x={})",
        a.bounds.x
    ));
    if let Some(cur) = state.anchors.lock().get_mut(w.label()) {
        cur.collapsed = true;
    }
    slide_window(w, x).await
}

/// 展开：滑回屏幕内贴平边缘。
#[tauri::command]
async fn snap_expand(
    w: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    side: snap::Side,
) -> CmdResult<()> {
    let size = w.outer_size().map_err(err)?;
    let a = anchor_for(&w, &state, side)?;
    let x = snap::expanded_x(a.bounds, side, size.width as i32);
    snap_log(&format!(
        "expand: side={side:?} -> x={x} (mon.x={})",
        a.bounds.x
    ));
    if let Some(cur) = state.anchors.lock().get_mut(w.label()) {
        cur.collapsed = false;
    }
    slide_window(w, x).await
}

/// 读回持久化的吸附边，浮窗启动时恢复。
#[tauri::command]
fn snap_restore(
    w: tauri::WebviewWindow,
    state: tauri::State<AppState>,
) -> CmdResult<Option<snap::Side>> {
    let key = if w.label() == "mini" {
        "mini_snap"
    } else {
        "main_snap"
    };
    Ok(match state.store.meta_get(key).map_err(err)?.as_deref() {
        Some("left") => Some(snap::Side::Left),
        Some("right") => Some(snap::Side::Right),
        _ => None,
    })
}

#[tauri::command]
fn get_autostart(app: tauri::AppHandle) -> CmdResult<bool> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(err)
}

#[tauri::command]
fn set_autostart(app: tauri::AppHandle, on: bool) -> CmdResult<()> {
    use tauri_plugin_autostart::ManagerExt;
    let al = app.autolaunch();
    if on {
        al.enable().map_err(err)
    } else {
        al.disable().map_err(err)
    }
}

#[tauri::command]
fn toggle_mini(app: tauri::AppHandle) -> CmdResult<()> {
    if let Some(w) = app.get_webview_window("mini") {
        let visible = w.is_visible().unwrap_or(false);
        if visible {
            w.hide().map_err(err)?;
        } else {
            w.show().map_err(err)?;
            let _ = w.set_always_on_top(true);
        }
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(
        &app,
        "mini",
        tauri::WebviewUrl::App("index.html?mini=1".into()),
    )
    .title("Lilo 浮窗")
    .inner_size(280.0, 120.0)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .build()
    .map_err(err)?;
    if let Some(mini) = app.get_webview_window("mini") {
        let percent = panel_opacity_percent(app.state::<AppState>().inner());
        apply_panel_opacity(&mini, percent);
    }
    Ok(())
}

/// 可选档：80 / 85 / 90 / 95 / 100。默认 90，80 在桌面上偏透。
const PANEL_OPACITY_PRESETS: [u8; 5] = [80, 85, 90, 95, 100];
const PANEL_OPACITY_DEFAULT: u8 = 90;

fn clamp_panel_opacity(percent: u8) -> u8 {
    PANEL_OPACITY_PRESETS
        .into_iter()
        .min_by_key(|p| p.abs_diff(percent))
        .unwrap_or(PANEL_OPACITY_DEFAULT)
}

fn panel_opacity_percent(state: &AppState) -> u8 {
    state
        .store
        .meta_get("panel_opacity")
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .map(clamp_panel_opacity)
        .unwrap_or(PANEL_OPACITY_DEFAULT)
}

fn apply_all_panel_opacity(app: &tauri::AppHandle, percent: u8) {
    for label in ["main", "mini"] {
        if let Some(w) = app.get_webview_window(label) {
            apply_panel_opacity(&w, percent);
        }
    }
}

/// Windows 上 WebView 不会把 CSS 半透明合成到桌面，只能用分层窗口透明度。
#[cfg(windows)]
fn apply_panel_opacity(window: &tauri::WebviewWindow, percent: u8) {
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetLayeredWindowAttributes, SetWindowLongPtrW, GWL_EXSTYLE, LWA_ALPHA,
        WS_EX_LAYERED,
    };
    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    let percent = clamp_panel_opacity(percent);
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if percent >= 100 {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex & !(WS_EX_LAYERED.0 as isize));
            return;
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as isize);
        let alpha = ((percent as u16 * 255) / 100) as u8;
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
    }
}

#[cfg(not(windows))]
fn apply_panel_opacity(_window: &tauri::WebviewWindow, _percent: u8) {}

#[tauri::command]
fn get_panel_opacity(state: tauri::State<AppState>) -> u8 {
    panel_opacity_percent(&state)
}

#[tauri::command]
fn set_panel_opacity(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    percent: u8,
) -> CmdResult<u8> {
    let percent = clamp_panel_opacity(percent);
    state
        .store
        .meta_set("panel_opacity", &percent.to_string())
        .map_err(err)?;
    apply_all_panel_opacity(&app, percent);
    Ok(percent)
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            use tauri::menu::{Menu, MenuItem};
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

            let show = MenuItem::with_id(app, "show", "打开 Lilo", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出 Lilo", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            TrayIconBuilder::with_id("lilo")
                .icon(app.default_window_icon().expect("应用图标缺失").clone())
                .tooltip("Lilo 待办")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        show_main_window(tray.app_handle());
                    }
                })
                .build(app)?;

            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let store = Arc::new(Store::open(&resolve_db_path(&dir))?);

            // 同步开关持久化，默认开
            let enabled = store
                .meta_get("sync_enabled")
                .ok()
                .flatten()
                .map(|v| v == "1")
                .unwrap_or(true);

            let lark = Arc::new(Lark::new());
            let sync = Sync::new(
                Arc::clone(&store),
                Arc::clone(&lark),
                app.handle().clone(),
                enabled,
            );

            sync.spawn_push_loop();

            app.manage(AppState {
                store,
                lark,
                sync,
                anchors: parking_lot::Mutex::new(HashMap::new()),
            });
            if let Some(main) = app.get_webview_window("main") {
                apply_panel_opacity(&main, panel_opacity_percent(app.state::<AppState>().inner()));
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    // 隐藏窗口会同时移除任务栏按钮，后台同步和托盘继续运行。
                    if window.hide().is_ok() {
                        api.prevent_close();
                    }
                    return;
                }
            }
            // 主窗口或浮窗被点到前台 = 用户在看面板，这时再拉飞书。
            if !matches!(event, tauri::WindowEvent::Focused(true)) {
                return;
            }
            let Some(state) = window.try_state::<AppState>() else {
                return;
            };
            let s = Arc::clone(&state.sync);
            std::thread::spawn(move || {
                s.pull_if_stale(20);
            });
        })
        .invoke_handler(tauri::generate_handler![
            list_tasks,
            list_subtasks,
            day_counts,
            create_task,
            update_task,
            delete_task,
            reorder_tasks,
            resolve_conflict,
            sync_status,
            sync_now,
            pull_on_view,
            set_sync_enabled,
            lark_check,
            lark_account,
            lark_grant_write,
            snap_debug,
            toggle_mini,
            snap_check,
            snap_to,
            snap_unsnap,
            snap_collapse,
            snap_expand,
            snap_restore,
            snap_pointer_over,
            get_autostart,
            set_autostart,
            get_panel_opacity,
            set_panel_opacity,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Lilo 失败");
}
