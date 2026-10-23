use serde::{Deserialize, Serialize};

/// 一条待办。本地 SQLite 是唯一真相源，飞书只是一个可选的同步目标。
///
/// `day` 为 `None` 表示这条待办在收集箱里（未排期）。飞书侧对应 `due` 为空。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub summary: String,
    #[serde(default)]
    pub description: String,
    /// 本地日期 "YYYY-MM-DD"；None = 收集箱
    pub day: Option<String>,
    /// 精确到分的提醒时刻（本地时区 epoch ms）。仅在 day 有值时可用。
    pub remind_at: Option<i64>,
    pub done: bool,
    pub completed_at: Option<i64>,
    /// 同一天内的手动排序位
    pub position: i64,
    pub parent_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,

    // ---- 同步状态，前端只读 ----
    pub lark_guid: Option<String>,
    /// 飞书任务详情页 applink
    pub lark_url: Option<String>,
    /// 与飞书冲突：两侧都改了同一字段，本地已保留
    #[serde(default)]
    pub conflict: bool,
}

/// 一次本地写入要提交给飞书的字段集合。用于字段级冲突消解 —— 拉取时
/// 不覆盖仍处于 dirty 的字段，避免「改了标题把对方的截止日期冲掉」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Summary,
    Description,
    Day,
    Remind,
    Done,
}

impl Field {
    pub fn as_str(self) -> &'static str {
        match self {
            Field::Summary => "summary",
            Field::Description => "description",
            Field::Day => "day",
            Field::Remind => "remind",
            Field::Done => "done",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "summary" => Field::Summary,
            "description" => Field::Description,
            "day" => Field::Day,
            "remind" => Field::Remind,
            "done" => Field::Done,
            _ => return None,
        })
    }
}

/// 前端新建待办的入参
#[derive(Debug, Clone, Deserialize)]
pub struct NewTask {
    pub summary: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub day: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
}

/// 前端更新待办的入参。字段为 None 表示不改动；
/// 用 `Option<Option<T>>` 区分「不改」与「清空」。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TaskPatch {
    pub summary: Option<String>,
    pub description: Option<String>,
    #[serde(default, with = "double_option")]
    pub day: Option<Option<String>>,
    #[serde(default, with = "double_option")]
    pub remind_at: Option<Option<i64>>,
    pub done: Option<bool>,
}

/// 让 serde 把显式的 `null` 解析成 `Some(None)`（清空），
/// 字段缺失解析成 `None`（不改动）。
mod double_option {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Option::<T>::deserialize(de).map(Some)
    }
}

/// 同步引擎状态，用于前端状态栏展示
#[derive(Debug, Clone, Serialize)]
pub struct SyncStatus {
    /// idle | syncing | offline | unauthorized | disabled
    pub state: String,
    pub last_sync_at: Option<i64>,
    pub pending: i64,
    pub message: Option<String>,
}
