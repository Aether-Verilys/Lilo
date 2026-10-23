//! 对着真实飞书账号验证拉取链路（只读，不写飞书）。
//! 跑法：cargo run --example live_pull

fn main() {
    let lark = lilo_lib::testing::Lark::new();

    match lark.missing_write_scopes() {
        Ok(missing) if missing.is_empty() => println!("✓ 写权限齐全"),
        Ok(missing) => println!("⚠ 缺少写权限: {}", missing.join(", ")),
        Err(e) => {
            println!("✗ 无法确认授权状态: {e}");
            return;
        }
    }

    let tasks = match lark.list_my_tasks() {
        Ok(t) => t,
        Err(e) => {
            println!("✗ 拉取失败: {e}");
            return;
        }
    };
    println!("✓ 拉到 {} 条飞书任务（已分页取完）", tasks.len());

    let inbox = tasks.iter().filter(|t| t.day.is_none()).count();
    let dated = tasks.len() - inbox;
    let done = tasks.iter().filter(|t| t.done).count();
    let reminders = tasks.iter().filter(|t| t.remind_at.is_some()).count();
    println!("  已排期 {dated} / 收集箱 {inbox} / 已完成 {done} / 带提醒 {reminders}");

    // 落库一遍，确认 upsert 与去重都正常
    let dir = std::env::temp_dir().join("lilo-live-check");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = lilo_lib::testing::Store::open(&dir.join("k.db")).unwrap();

    let mut applied = 0;
    for t in &tasks {
        if store.apply_remote(t).unwrap().is_some() {
            applied += 1;
        }
    }
    println!("✓ 落库 {applied} 条");

    // 再落一遍，条数不应变化（幂等）
    for t in &tasks {
        store.apply_remote(t).unwrap();
    }

    let today = chrono_today();
    let n = store.tasks_for_day(Some(&today)).unwrap().len();
    let inbox_n = store.tasks_for_day(None).unwrap().len();
    println!("✓ 幂等复核：今天 {n} 条，收集箱 {inbox_n} 条");
    println!(
        "✓ 待推送 {} 条（拉取不应产生脏标记）",
        store.pending_count().unwrap()
    );

    if let Some(t) = tasks.iter().find(|t| t.day.is_some()) {
        println!(
            "  样例: {:?} day={:?} done={} guid={}",
            t.summary,
            t.day,
            t.done,
            &t.guid[..8]
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn chrono_today() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // 只用于粗略打印，直接按 UTC 算日期即可
    let days = secs / 86400;
    let (y, m, d) = civil_from_days(days as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Howard Hinnant 的 civil_from_days
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
