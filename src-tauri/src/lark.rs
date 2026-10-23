use crate::model::Field;
use crate::store::RemoteTask;
use chrono::{Local, NaiveDate, TimeZone};
use serde_json::{json, Value};
use std::io;
use std::process::{Command, Stdio};

#[derive(Debug, thiserror::Error)]
pub enum LarkError {
    #[error("lark-cli 未安装或不在 PATH 中")]
    NotInstalled,
    #[error("未授权或授权已过期")]
    Unauthorized,
    #[error("缺少权限：{0}")]
    MissingScope(String),
    #[error("需要二次确认的高风险操作：{0}")]
    ConfirmationRequired(String),
    #[error("网络不可用")]
    Offline,
    #[error("lark-cli 报错：{0}")]
    Cli(String),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("解析 lark-cli 输出失败：{0}")]
    Parse(String),
}

pub type Result<T> = std::result::Result<T, LarkError>;

/// 通过 lark-cli 子进程访问飞书任务。
///
/// 一律用 argv 数组调用，从不经过 shell —— 待办标题里的引号、`&&`、`$()`
/// 都只会被当成普通字符串传递。
pub struct Lark {
    program: String,
}

impl Lark {
    pub fn new() -> Self {
        Self {
            program: resolve_program(),
        }
    }

    /// 允许用户在设置里指定 lark-cli 的绝对路径（未装在 PATH 里的情况）
    pub fn with_program(program: String) -> Self {
        Self { program }
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    /// API 调用：要求 `ok == true` 信封。
    fn run(&self, args: &[&str]) -> Result<Value> {
        let v = self.raw(args)?;
        // 成功判定必须看 ok == true。成功信封没有顶层 code 字段，
        // 按 {"code":0} 判断会把所有成功调用误判为失败 —— 写入类命令上
        // 这种误判会绕过幂等逻辑导致重复创建。
        if v.get("ok").and_then(Value::as_bool) != Some(true) {
            return Err(LarkError::Cli(truncate(&v.to_string(), 300)));
        }
        Ok(v)
    }

    /// CLI 管理类命令（如 `auth status`）直接返回裸 payload，没有 ok 信封。
    fn raw(&self, args: &[&str]) -> Result<Value> {
        let mut command = Command::new(&self.program);
        command.args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let out = command
            .env("LARKSUITE_CLI_NO_UPDATE_NOTIFIER", "1")
            .env("LARKSUITE_CLI_NO_SKILLS_NOTIFIER", "1")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| {
                if e.kind() == io::ErrorKind::NotFound {
                    LarkError::NotInstalled
                } else {
                    LarkError::Io(e)
                }
            })?;

        // exit 10 = 高风险操作待确认，必须原样上报，不能自动补 --yes
        if out.status.code() == Some(10) {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let action = serde_json::from_str::<Value>(&stderr)
                .ok()
                .and_then(|v| v.get("error")?.get("action")?.as_str().map(str::to_string))
                .unwrap_or_else(|| args.join(" "));
            return Err(LarkError::ConfirmationRequired(action));
        }

        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(classify_error(&stderr));
        }

        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        parse_cli_json(&stdout, &stderr)
    }

    pub fn auth_ok(&self) -> bool {
        self.raw(&["auth", "status", "--json"]).is_ok()
    }

    /// 当前已授权的飞书用户，设置页用来展示账号。
    pub fn account(&self) -> Result<LarkAccount> {
        let v = unwrap_payload(self.raw(&["auth", "status", "--json"])?);
        let user = v.pointer("/identities/user").unwrap_or(&Value::Null);
        let status = user.get("status").and_then(Value::as_str).unwrap_or("");
        let token = user
            .get("tokenStatus")
            .and_then(Value::as_str)
            .unwrap_or("");
        Ok(LarkAccount {
            name: user
                .get("userName")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            open_id: user
                .get("openId")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            ready: status == "ready" && (token == "valid" || token == "ready"),
        })
    }

    /// 当前 user 身份是否已拿到任务读写 scope
    pub fn missing_write_scopes(&self) -> Result<Vec<String>> {
        let v = unwrap_payload(self.raw(&["auth", "status", "--json"])?);
        let scope = v
            .pointer("/identities/user/scope")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let granted: Vec<&str> = scope.split_whitespace().collect();
        Ok(["task:task:write", "task:tasklist:write"]
            .iter()
            .filter(|s| !granted.contains(*s))
            .map(|s| s.to_string())
            .collect())
    }

    /// 发起写入权限的设备授权，立即返回扫码链接，不阻塞。
    pub fn start_login(&self, scopes: &[String]) -> Result<LoginPending> {
        let mut args: Vec<String> = vec![
            "auth".into(),
            "login".into(),
            "--no-wait".into(),
            "--json".into(),
        ];
        if !scopes.is_empty() {
            args.push("--scope".into());
            args.push(scopes.join(","));
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let v = self.raw(&refs)?;
        parse_login_pending(&v).ok_or_else(|| {
            LarkError::Parse(format!(
                "授权响应缺少 device_code / verification_url: {}",
                truncate(&v.to_string(), 200)
            ))
        })
    }

    /// 用上一步的 device_code 等到用户在浏览器里点完同意。
    pub fn complete_login(&self, device_code: &str) -> Result<()> {
        match self.raw(&["auth", "login", "--device-code", device_code, "--json"]) {
            Ok(_) | Err(LarkError::Parse(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// 拉取「我负责的」任务。分页取完。
    pub fn list_my_tasks(&self) -> Result<Vec<RemoteTask>> {
        let mut out = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let token = page_token.take();
            let mut args: Vec<&str> = vec![
                "task",
                "tasks",
                "list",
                "--as",
                "user",
                "--page-size",
                "100",
            ];
            if let Some(t) = token.as_deref() {
                args.push("--page-token");
                args.push(t);
            }
            let v = self.run(&args)?;
            let data = v
                .get("data")
                .ok_or_else(|| LarkError::Parse("响应缺少 data 字段".to_string()))?;

            if let Some(items) = data.get("items").and_then(Value::as_array) {
                for it in items {
                    if let Some(rt) = parse_remote(it) {
                        out.push(rt);
                    }
                }
            }

            let has_more = data.get("has_more").and_then(Value::as_bool) == Some(true);
            let next = data.get("page_token").and_then(Value::as_str);
            match (has_more, next) {
                (true, Some(t)) if !t.is_empty() => page_token = Some(t.to_string()),
                _ => break,
            }
        }
        Ok(out)
    }

    pub fn get_task(&self, guid: &str) -> Result<Option<RemoteTask>> {
        let v = self.run(&["task", "tasks", "get", "--task-guid", guid, "--as", "user"])?;
        Ok(v.pointer("/data/task").and_then(parse_remote))
    }

    /// 新建飞书任务，返回 (guid, url, updated_at)。
    /// `local_id` 作为 client_token 保证幂等 —— 重试不会重复建任务。
    pub fn create_task(
        &self,
        local_id: &str,
        summary: &str,
        description: &str,
        day: Option<&str>,
        remind_at: Option<i64>,
    ) -> Result<(String, Option<String>, i64)> {
        let mut task = json!({
            "summary": summary,
            "description": description,
            "client_token": local_id,
        });
        if let Some(d) = day {
            if let Some(ts) = day_to_timestamp(d) {
                task["due"] = json!({ "timestamp": ts.to_string(), "is_all_day": true });
                // 飞书要求先有 due 才能设 reminder
                if let Some(r) = remind_at {
                    if let Some(mins) = relative_fire_minute(d, r) {
                        task["reminders"] = json!([{ "relative_fire_minute": mins }]);
                    }
                }
            }
        }
        let body = task.to_string();
        let v = self.run(&["task", "tasks", "create", "--as", "user", "--data", &body])?;
        let t = v
            .pointer("/data/task")
            .ok_or_else(|| LarkError::Parse("创建响应缺少 data.task".to_string()))?;
        let guid = t
            .get("guid")
            .and_then(Value::as_str)
            .ok_or_else(|| LarkError::Parse("创建响应缺少 guid".to_string()))?
            .to_string();
        let url = t.get("url").and_then(Value::as_str).map(str::to_string);
        let updated = t
            .get("updated_at")
            .and_then(parse_ms)
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
        Ok((guid, url, updated))
    }

    /// 按字段增量更新飞书任务。只提交 `fields` 里的字段。
    pub fn patch_task(
        &self,
        guid: &str,
        fields: &[Field],
        summary: &str,
        description: &str,
        day: Option<&str>,
        remind_at: Option<i64>,
        done: bool,
        completed_at: Option<i64>,
    ) -> Result<i64> {
        let mut task = json!({});
        let mut update_fields: Vec<&str> = Vec::new();

        for f in fields {
            match f {
                Field::Summary => {
                    task["summary"] = json!(summary);
                    update_fields.push("summary");
                }
                Field::Description => {
                    task["description"] = json!(description);
                    update_fields.push("description");
                }
                Field::Day => {
                    match day.and_then(day_to_timestamp) {
                        Some(ts) => {
                            task["due"] =
                                json!({ "timestamp": ts.to_string(), "is_all_day": true });
                        }
                        // update_fields 列了字段但 task 里不给值 = 清空
                        None => {}
                    }
                    update_fields.push("due");
                }
                Field::Remind => {
                    // reminders 依赖 due 存在；收集箱任务不带提醒
                    if let (Some(d), Some(r)) = (day, remind_at) {
                        if let Some(mins) = relative_fire_minute(d, r) {
                            task["reminders"] = json!([{ "relative_fire_minute": mins }]);
                        }
                    } else {
                        task["reminders"] = json!([]);
                    }
                    update_fields.push("reminders");
                }
                Field::Done => {
                    if done && self.remote_already_done(guid) {
                        // 飞书拒绝给已完成任务再写非零 completed_at。
                        // 本地再点一次完成时，跳过这个字段，当成已经对齐。
                        continue;
                    }
                    let ts = if done {
                        completed_at.unwrap_or_else(|| chrono::Utc::now().timestamp_millis())
                    } else {
                        0
                    };
                    task["completed_at"] = json!(ts.to_string());
                    update_fields.push("completed_at");
                }
            }
        }

        if update_fields.is_empty() {
            return Ok(self
                .get_task(guid)?
                .map(|t| t.updated_at)
                .unwrap_or_else(|| chrono::Utc::now().timestamp_millis()));
        }

        let body = json!({ "task": task, "update_fields": update_fields }).to_string();
        let v = match self.run(&[
            "task",
            "tasks",
            "patch",
            "--task-guid",
            guid,
            "--as",
            "user",
            "--data",
            &body,
        ]) {
            Ok(v) => v,
            // GET 与 PATCH 之间远端可能刚被完成，按幂等成功处理，避免脏标记卡死。
            Err(e) if already_completed_reject(&e) => {
                return Ok(self
                    .get_task(guid)?
                    .map(|t| t.updated_at)
                    .unwrap_or_else(|| chrono::Utc::now().timestamp_millis()));
            }
            Err(e) => return Err(e),
        };
        Ok(v.pointer("/data/task/updated_at")
            .and_then(parse_ms)
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis()))
    }

    fn remote_already_done(&self, guid: &str) -> bool {
        matches!(self.get_task(guid), Ok(Some(t)) if t.done)
    }
}

impl Default for Lark {
    fn default() -> Self {
        Self::new()
    }
}

/// 找到可执行的 lark-cli。
///
/// Windows 上 npm 全局装的是 `lark-cli.cmd` 批处理壳，Rust 的 `Command`
/// 不会像 shell 那样按 PATHEXT 去补后缀，直接传 "lark-cli" 会 NotFound。
/// 优先找包内自带的原生 exe，其次才退回名字让 PATH 解析。
fn resolve_program() -> String {
    // 允许用环境变量覆盖，方便打包版指定路径
    if let Ok(p) = std::env::var("LILO_LARK_CLI").or_else(|_| std::env::var("KITE_LARK_CLI")) {
        if !p.trim().is_empty() {
            return p;
        }
    }

    #[cfg(windows)]
    {
        let mut roots: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(appdata) = std::env::var("APPDATA") {
            roots.push(std::path::PathBuf::from(appdata).join("npm"));
        }
        for root in roots {
            // 包内原生 exe，直接可被 Command 执行
            let exe = root
                .join("node_modules")
                .join("@larksuite")
                .join("cli")
                .join("bin")
                .join("lark-cli.exe");
            if exe.is_file() {
                return exe.to_string_lossy().into_owned();
            }
        }
        // 退而求其次找 PATH 里的 .exe / .cmd
        if let Ok(path) = std::env::var("PATH") {
            for dir in std::env::split_paths(&path) {
                for name in ["lark-cli.exe", "lark-cli.cmd"] {
                    let p = dir.join(name);
                    if p.is_file() {
                        return p.to_string_lossy().into_owned();
                    }
                }
            }
        }
    }

    "lark-cli".to_string()
}

/// 飞书不允许给已完成任务再写非零 completed_at。
/// 本地再点完成时必须跳过，否则会一直报 `lark-cli 报错`。
pub fn omit_completed_at_patch(want_done: bool, remote_already_done: bool) -> bool {
    want_done && remote_already_done
}

pub fn is_already_completed_reject(msg: &str) -> bool {
    msg.contains("cannot set non-zero completed_at for a completed task")
}

fn already_completed_reject(e: &LarkError) -> bool {
    match e {
        LarkError::Cli(m) => is_already_completed_reject(m),
        _ => false,
    }
}

fn classify_error(stderr: &str) -> LarkError {
    let Ok(v) = serde_json::from_str::<Value>(stderr) else {
        let low = stderr.to_lowercase();
        if low.contains("network") || low.contains("dial") || low.contains("timeout") {
            return LarkError::Offline;
        }
        return LarkError::Cli(truncate(stderr, 300));
    };
    let err = v.get("error").unwrap_or(&Value::Null);
    let ty = err.get("type").and_then(Value::as_str).unwrap_or("");
    let subtype = err.get("subtype").and_then(Value::as_str).unwrap_or("");

    if let Some(scopes) = err.get("missing_scopes").and_then(Value::as_array) {
        let list: Vec<String> = scopes
            .iter()
            .filter_map(|s| s.as_str().map(str::to_string))
            .collect();
        if !list.is_empty() {
            return LarkError::MissingScope(list.join(", "));
        }
    }
    match (ty, subtype) {
        ("authorization", _) | ("authentication", _) => LarkError::Unauthorized,
        ("network", _) => LarkError::Offline,
        _ => LarkError::Cli(match err.get("message").and_then(Value::as_str) {
            Some(m) => m.to_string(),
            None => truncate(stderr, 300),
        }),
    }
}

/// 供集成测试对着真实响应结构验证映射
pub fn parse_remote_for_test(v: &Value) -> Option<RemoteTask> {
    parse_remote(v)
}

/// 飞书把 epoch ms 编成字符串下发，偶尔也有数字形式
fn parse_ms(v: &Value) -> Option<i64> {
    match v {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

/// 把飞书任务归一化成本地语义。
/// 关键点：`due` 为空 → 收集箱；`guid` 才是 API 标识，`task_id`（t114038）只是界面编号。
fn parse_remote(v: &Value) -> Option<RemoteTask> {
    let guid = v.get("guid")?.as_str()?.to_string();
    let summary = v
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let description = v
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let due_ts = v.pointer("/due/timestamp").and_then(parse_ms);
    let day = due_ts.and_then(timestamp_to_day);

    let completed_at = v.get("completed_at").and_then(parse_ms).filter(|&t| t > 0);
    // status 是 "todo" / "done"；completed_at > 0 是更可靠的兜底
    let done = v.get("status").and_then(Value::as_str) == Some("done") || completed_at.is_some();

    let remind_at = v
        .get("reminders")
        .and_then(Value::as_array)
        .and_then(|rs| rs.first())
        .and_then(|r| r.get("relative_fire_minute").and_then(Value::as_i64))
        .zip(due_ts)
        .map(|(mins, due)| due - mins * 60_000);

    let url = v.get("url").and_then(Value::as_str).map(str::to_string);
    let updated_at = v.get("updated_at").and_then(parse_ms).unwrap_or(0);

    Some(RemoteTask {
        guid,
        summary,
        description,
        day,
        remind_at,
        done,
        completed_at,
        url,
        updated_at,
    })
}

/// "YYYY-MM-DD" → 当地当天 00:00 的 epoch ms
fn day_to_timestamp(day: &str) -> Option<i64> {
    let d = NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?;
    let dt = d.and_hms_opt(0, 0, 0)?;
    Local
        .from_local_datetime(&dt)
        .earliest()
        .map(|t| t.timestamp_millis())
}

/// epoch ms → 当地日期 "YYYY-MM-DD"
fn timestamp_to_day(ts: i64) -> Option<String> {
    Local
        .timestamp_millis_opt(ts)
        .single()
        .map(|t| t.format("%Y-%m-%d").to_string())
}

/// 飞书的提醒是「相对截止时间提前 N 分钟」，本地存的是绝对时刻，这里换算。
fn relative_fire_minute(day: &str, remind_at: i64) -> Option<i64> {
    let due = day_to_timestamp(day)?;
    Some((due - remind_at) / 60_000)
}

/// 当前已授权的飞书用户
#[derive(Debug, Clone, serde::Serialize)]
pub struct LarkAccount {
    pub name: Option<String>,
    pub open_id: Option<String>,
    pub ready: bool,
}

/// 设备授权第一步的返回值
#[derive(Debug, Clone)]
pub struct LoginPending {
    pub device_code: String,
    pub verification_url: String,
}

fn unwrap_payload(v: Value) -> Value {
    if v.get("ok").and_then(Value::as_bool) == Some(true) {
        if let Some(data) = v.get("data") {
            return data.clone();
        }
    }
    v
}

fn json_str(v: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = v.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty()) {
            return Some(s.to_string());
        }
        let path = format!("/data/{k}");
        if let Some(s) = v
            .pointer(&path)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            return Some(s.to_string());
        }
    }
    None
}

fn parse_login_pending(v: &Value) -> Option<LoginPending> {
    let v = unwrap_payload(v.clone());
    let device_code = json_str(&v, &["device_code"])?;
    let verification_url = json_str(
        &v,
        &[
            "verification_uri_complete",
            "verification_url",
            "verification_uri",
        ],
    )?;
    Some(LoginPending {
        device_code,
        verification_url,
    })
}

/// 供测试解析 `auth login --no-wait --json` 的响应结构
pub fn parse_login_pending_for_test(v: &Value) -> Option<(String, String)> {
    parse_login_pending(v).map(|p| (p.device_code, p.verification_url))
}

fn parse_cli_json(stdout: &str, stderr: &str) -> Result<Value> {
    if let Some(v) = extract_json(stdout) {
        return Ok(v);
    }
    if let Some(v) = extract_json(stderr) {
        return Ok(v);
    }
    if stdout.trim().is_empty() && stderr.trim().is_empty() {
        return Ok(json!({}));
    }
    Err(LarkError::Parse(format!(
        "无法解析 JSON: {}",
        truncate(
            if stdout.trim().is_empty() {
                stderr
            } else {
                stdout
            },
            200
        )
    )))
}

fn extract_json(s: &str) -> Option<Value> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return Some(v);
    }
    let start = t.find('{')?;
    let end = t.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&t[start..=end]).ok()
}

fn truncate(s: &str, n: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= n {
        return t.to_string();
    }
    t.chars().take(n).collect::<String>() + "…"
}
