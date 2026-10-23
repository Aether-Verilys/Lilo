//! 验证飞书任务 → 本地语义的映射，以及本地存储的冲突消解规则。
//!
//! 用的是从真实 `lark-cli task tasks list` 抓下来的响应结构，
//! 确保字段名和类型（尤其是 epoch ms 被编成字符串这点）没猜错。

use lilo_lib::testing::{
    is_already_completed_reject, omit_completed_at_patch, parse_login_pending_for_test,
    parse_remote_for_test, Store,
};
use lilo_lib::testing::{Field, NewTask, TaskPatch};

const REAL_TASK: &str = r#"{
  "completed_at": "0",
  "created_at": "1788625579470",
  "creator": { "id": "ou_9127", "type": "user" },
  "description": "【下周任务】FBX 白皮书更新",
  "due": { "is_all_day": true, "timestamp": "1789084800000" },
  "guid": "73039343-ba8e-4ea7-b3b2-d8491f75e148",
  "members": [{ "id": "ou_9127", "role": "assignee", "type": "user" }],
  "status": "todo",
  "subtask_count": 0,
  "summary": "fbx白皮书更新",
  "task_id": "t114038",
  "updated_at": "1788625627639",
  "url": "https://applink.feishu.cn/client/todo/detail?guid=73039343-ba8e-4ea7-b3b2-d8491f75e148"
}"#;

#[test]
fn skips_completed_at_when_remote_already_done() {
    assert!(omit_completed_at_patch(true, true));
    assert!(!omit_completed_at_patch(true, false));
    assert!(!omit_completed_at_patch(false, true));
    assert!(!omit_completed_at_patch(false, false));
}

#[test]
fn detects_feishu_already_completed_reject() {
    let msg = "Invalid Param 'task.completed_at', cannot set non-zero completed_at for a completed task. To change task's completed_at, you can update task's 'completed_at' to 0, then update its 'completed'_at to new value.";
    assert!(is_already_completed_reject(msg));
    assert!(!is_already_completed_reject("missing_scope"));
}

#[test]
fn parses_real_task_response() {
    let v: serde_json::Value = serde_json::from_str(REAL_TASK).unwrap();
    let r = parse_remote_for_test(&v).expect("应该能解析真实任务");

    assert_eq!(r.guid, "73039343-ba8e-4ea7-b3b2-d8491f75e148");
    assert_eq!(r.summary, "fbx白皮书更新");
    // updated_at 是字符串形式的 epoch ms
    assert_eq!(r.updated_at, 1788625627639);
    // completed_at 为 "0" 表示未完成，不能当成 1970 年完成
    assert!(!r.done);
    assert_eq!(r.completed_at, None);
    // due 存在 → 落到某一天，不是收集箱
    assert!(r.day.is_some(), "有 due 的任务应该有 day");
}

#[test]
fn parses_device_login_pending() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"device_code":"dc-1","verification_url":"https://accounts.feishu.cn/oauth/v1/device?user_code=ABCD"}"#,
    )
    .unwrap();
    let (code, url) = parse_login_pending_for_test(&v).expect("应解析出 device_code 和 url");
    assert_eq!(code, "dc-1");
    assert!(url.starts_with("https://"));
}

#[test]
fn parses_device_login_pending_in_ok_envelope() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"ok":true,"data":{"device_code":"dc-2","verification_uri_complete":"https://open.feishu.cn/device?user_code=WXYZ"}}"#,
    )
    .unwrap();
    let (code, url) = parse_login_pending_for_test(&v).unwrap();
    assert_eq!(code, "dc-2");
    assert!(url.contains("user_code=WXYZ"));
}

#[test]
fn task_without_due_goes_to_inbox() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"guid":"g1","summary":"没有截止日期","updated_at":"100","status":"todo"}"#,
    )
    .unwrap();
    let r = parse_remote_for_test(&v).unwrap();
    assert_eq!(r.day, None, "没有 due 的任务应进收集箱");
}

#[test]
fn done_detected_from_completed_at_even_if_status_missing() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"guid":"g2","summary":"完成了","completed_at":"1788625627639","updated_at":"100"}"#,
    )
    .unwrap();
    let r = parse_remote_for_test(&v).unwrap();
    assert!(r.done, "completed_at > 0 应判定为已完成");
    assert_eq!(r.completed_at, Some(1788625627639));
}

fn tmp_store() -> (Store, tempdir::Guard) {
    let g = tempdir::Guard::new();
    let s = Store::open(&g.path().join("t.db")).unwrap();
    (s, g)
}

#[test]
fn new_task_is_marked_dirty_for_push() {
    let (s, _g) = tmp_store();
    let t = s
        .create(&NewTask {
            summary: "写周报".into(),
            description: String::new(),
            day: Some("2026-09-08".into()),
            parent_id: None,
        })
        .unwrap();
    let dirty = s.dirty_fields(&t.id).unwrap();
    assert!(dirty.contains(&Field::Summary), "新任务应待推送 summary");
    assert!(dirty.contains(&Field::Day), "新任务应待推送 day");
    assert_eq!(s.pending_count().unwrap(), 1);
}

#[test]
fn moving_to_inbox_clears_reminder() {
    // 飞书要求先有 due 才能有 reminder，所以挪进收集箱必须清掉提醒，
    // 否则推送会被服务端拒绝。
    let (s, _g) = tmp_store();
    let t = s
        .create(&NewTask {
            summary: "开会".into(),
            description: String::new(),
            day: Some("2026-09-08".into()),
            parent_id: None,
        })
        .unwrap();
    s.update(
        &t.id,
        &TaskPatch {
            remind_at: Some(Some(1789084800000)),
            ..Default::default()
        },
    )
    .unwrap();
    let t = s.get(&t.id).unwrap().unwrap();
    assert!(t.remind_at.is_some(), "有 day 时应能设提醒");

    s.update(
        &t.id,
        &TaskPatch {
            day: Some(None),
            ..Default::default()
        },
    )
    .unwrap();
    let t = s.get(&t.id).unwrap().unwrap();
    assert_eq!(t.day, None);
    assert_eq!(t.remind_at, None, "挪进收集箱后提醒必须被清掉");
}

#[test]
fn inbox_task_rejects_reminder() {
    let (s, _g) = tmp_store();
    let t = s
        .create(&NewTask {
            summary: "随手记".into(),
            description: String::new(),
            day: None,
            parent_id: None,
        })
        .unwrap();
    s.update(
        &t.id,
        &TaskPatch {
            remind_at: Some(Some(1789084800000)),
            ..Default::default()
        },
    )
    .unwrap();
    let t = s.get(&t.id).unwrap().unwrap();
    assert_eq!(t.remind_at, None, "收集箱任务不应接受提醒");
}

#[test]
fn remote_pull_does_not_clobber_unpushed_local_edit() {
    // 核心冲突场景：本地改了标题还没推，飞书那边也改了标题。
    // 本地值必须保住，并打上冲突标记。
    let (s, _g) = tmp_store();
    let remote = lilo_lib::testing::RemoteTask {
        guid: "g9".into(),
        summary: "飞书原标题".into(),
        description: String::new(),
        day: Some("2026-09-08".into()),
        remind_at: None,
        done: false,
        completed_at: None,
        url: None,
        updated_at: 1000,
    };
    let id = s.apply_remote(&remote).unwrap().unwrap();

    // 本地改标题（产生脏标记），但还没推
    s.update(
        &id,
        &TaskPatch {
            summary: Some("我改的标题".into()),
            ..Default::default()
        },
    )
    .unwrap();

    // 飞书侧也改了，且时间更新
    let mut newer = remote.clone();
    newer.summary = "飞书改的标题".into();
    newer.description = "飞书加的描述".into();
    newer.updated_at = 2000;
    s.apply_remote(&newer).unwrap();

    let t = s.get(&id).unwrap().unwrap();
    assert_eq!(t.summary, "我改的标题", "未推送的本地改动不能被覆盖");
    assert_eq!(t.description, "飞书加的描述", "没冲突的字段应该正常更新");
    assert!(t.conflict, "应标记冲突让用户裁决");
}

#[test]
fn tombstone_prevents_deleted_task_from_returning() {
    let (s, _g) = tmp_store();
    let remote = lilo_lib::testing::RemoteTask {
        guid: "g-del".into(),
        summary: "要删的".into(),
        description: String::new(),
        day: None,
        remind_at: None,
        done: false,
        completed_at: None,
        url: None,
        updated_at: 1000,
    };
    let id = s.apply_remote(&remote).unwrap().unwrap();
    s.delete(&id).unwrap();

    // 再次拉到同一条：必须被墓碑拦住
    assert_eq!(
        s.apply_remote(&remote).unwrap(),
        None,
        "已删任务不应被同步回来"
    );
}

#[test]
fn clear_dirty_records_lark_identity() {
    let (s, _g) = tmp_store();
    let t = s
        .create(&NewTask {
            summary: "推一条".into(),
            description: String::new(),
            day: None,
            parent_id: None,
        })
        .unwrap();
    let fields = s.dirty_fields(&t.id).unwrap();
    s.clear_dirty(
        &t.id,
        &fields,
        Some("new-guid"),
        Some("https://applink/x"),
        Some(555),
    )
    .unwrap();
    assert_eq!(s.pending_count().unwrap(), 0, "推送成功后应无待推送");
    let t = s.get(&t.id).unwrap().unwrap();
    assert_eq!(t.lark_guid.as_deref(), Some("new-guid"));
}

/// 最小临时目录，避免为测试引入额外依赖
mod tempdir {
    use std::path::{Path, PathBuf};

    pub struct Guard(PathBuf);

    impl Guard {
        pub fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "lilo-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&p).unwrap();
            Guard(p)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
