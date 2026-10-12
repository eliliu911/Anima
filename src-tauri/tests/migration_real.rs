//! 对着**真实的 memory.db 副本**跑一遍迁移。
//!
//! 为什么必须这样验：`emotion` 是单行表且带 `CHECK (id = 1)`，SQLite 删不掉
//! CHECK 约束，所以只能建新表再搬。那一行里是用户攒了很久的好感度 ——
//! **丢了等于把关系清零**，而那正是分档设计要避免的事。
//!
//! 单元测试用的是空库，验不了"老数据能不能搬过来"。所以这里复制一份真实库。

/// 真实库能被迁移，而且好感度一分不差。
#[tokio::test]
#[ignore = "需要本机已有 memory.db（跑过语音对话）"]
async fn real_database_migrates_without_losing_affection() {
    let home = std::env::var("HOME").expect("取不到 HOME");
    let real = std::path::PathBuf::from(&home)
        .join("Library/Application Support/com.anima.desktop/memory.db");
    if !real.is_file() {
        println!("本机还没有 memory.db，跳过");
        return;
    }

    // 复制到临时目录再迁移 —— **绝不动用户的真库**
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("memory.db");
    std::fs::copy(&real, &copy).expect("复制失败");

    // 迁移前：从老表里读出好感度
    let before: (f64, f64, f64) = {
        let conn = rusqlite::Connection::open(&copy).unwrap();
        conn.query_row(
            "SELECT affection, mood, energy FROM emotion WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("老库里应当有那一行")
    };
    println!(
        "迁移前（老 emotion 表）：好感度 {:.1}  心情 {:.1}  活跃 {:.1}",
        before.0, before.1, before.2
    );

    // 打开 Store 会跑 migrate()
    let store = anima_lib::memory::store::Store::open(&copy).expect("打开失败");

    let after = store.emotion().await.expect("读取失败");
    println!(
        "迁移后（emotion_scoped/global）：好感度 {:.1}  心情 {:.1}  活跃 {:.1}",
        after.affection, after.mood, after.energy
    );

    assert!(
        (after.affection as f64 - before.0).abs() < 0.01,
        "好感度必须一分不差地搬过来：{} → {}",
        before.0,
        after.affection
    );
    assert!(
        (after.mood as f64 - before.1).abs() < 0.01,
        "心情也该搬过来"
    );
    assert!(
        (after.energy as f64 - before.2).abs() < 0.01,
        "活跃度也该搬过来"
    );

    // 再打开一次：不该重复迁移或覆盖
    drop(store);
    let store2 = anima_lib::memory::store::Store::open(&copy).expect("二次打开失败");
    let again = store2.emotion().await.expect("读取失败");
    assert_eq!(
        again.affection, after.affection,
        "二次打开不该改动已迁移的数据"
    );
    println!("二次打开：好感度仍是 {:.1}", again.affection);
}
