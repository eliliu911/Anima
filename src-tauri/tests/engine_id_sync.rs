//! 前端传的引擎 id 必须是 Rust 认得的。
//!
//! `SheetEngine::from_id` 认不出来时**不报错，静默回到 SDXL** ——
//! 这是故意的（用户升级后旧配置不该炸），但代价是前端把 `"sd15"` 写成
//! `"sd1_5"` 之后，界面上选着 SD1.5、后台一直跑 SDXL，谁都看不出来。
//! 所以两边的字面量在这里对一次。

use anima_lib::cinema_sheet::SheetEngine;
use std::path::PathBuf;

fn read(rel: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("读不到 {}: {e}", p.display()))
}

#[test]
fn 前端的引擎id_rust都认得() {
    let api = read("src/api/cinema.ts");

    // 从 `export type SheetEngine = "sdxl" | "sd15";` 里把字面量抠出来
    let line = api
        .lines()
        .find(|l| l.contains("export type SheetEngine"))
        .expect("src/api/cinema.ts 里应该有 SheetEngine 类型");
    let ids: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
    assert!(ids.len() >= 2, "至少该有两条路：{line}");

    for id in &ids {
        // from_id 认不出会静默回到 Sdxl，所以只看 sdxl 那条不够 ——
        // 得确认每个 id 都映射到**不同**的引擎
        let got = SheetEngine::from_id(id);
        eprintln!("  {id} -> {got:?}");
    }
    let mapped: std::collections::HashSet<_> = ids
        .iter()
        .map(|id| format!("{:?}", SheetEngine::from_id(id)))
        .collect();
    assert_eq!(
        mapped.len(),
        ids.len(),
        "有 id 被映射到了同一个引擎 —— 界面上选得到、后台不生效：{ids:?} -> {mapped:?}"
    );
}
