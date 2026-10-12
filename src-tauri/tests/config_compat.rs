//! 老版本写下的 config.json 必须仍然读得出来。
//!
//! 这条不是理论问题：TtsEngineKind 改名之后，用户机器上那份配置
//! 立刻整份解析失败、所有设置退回默认值。schema 改动的杀伤范围
//! 从来不止改动的那个字段。

/// 改名前（open_ai_compatible / gpt_so_vits）写下的配置片段
const LEGACY: &str = r#"{
  "providers": [],
  "realtime_provider": "lmstudio",
  "background_provider": "ollama",
  "disable_thinking": false,
  "tts_engines": [
    { "id": "edge", "label": "Edge 朗读（云端）", "kind": "edge", "base_url": "" },
    { "id": "openai_compatible", "label": "OpenAI 兼容（本地）",
      "kind": "open_ai_compatible", "base_url": "http://127.0.0.1:8880" },
    { "id": "gpt_sovits", "label": "GPT-SoVITS（本地克隆）",
      "kind": "gpt_so_vits", "base_url": "http://127.0.0.1:9880" }
  ],
  "tts_engine": "edge"
}"#;

#[test]
fn config_written_before_the_rename_still_parses() {
    let parsed: serde_json::Value = serde_json::from_str(LEGACY).unwrap();
    let engines = parsed["tts_engines"].as_array().unwrap();

    for e in engines {
        let kind: anima_lib::tts::TtsEngineKind = serde_json::from_value(e["kind"].clone())
            .unwrap_or_else(|err| panic!("旧 kind 值 {} 解析失败：{err}", e["kind"]));
        // 再序列化时会写成新名字，所以老文件会自动被"就地升级"
        let round = serde_json::to_string(&kind).unwrap();
        assert!(
            !round.contains("open_ai_compatible"),
            "应当写回新名字：{round}"
        );
        assert!(!round.contains("gpt_so_vits"), "应当写回新名字：{round}");
    }
}

/// 每一个新加的 TTS 引擎都必须能被 `merge_new_defaults` 补进老配置。
///
/// 这条踩过：老 config.json 里只有旧的三个引擎，新加的在设置面板里**根本
/// 不出现**，看起来像功能没做。上面那份 LEGACY 就是"只有三个引擎"的样子。
#[test]
fn newly_added_engines_appear_in_an_old_config() {
    let defaults = anima_lib::tts::TtsEngineConfig::defaults();
    let legacy: serde_json::Value = serde_json::from_str(LEGACY).unwrap();
    let have: Vec<String> = legacy["tts_engines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect();

    // 默认列表里比老配置多出来的那些，正是必须被补进去的
    let missing: Vec<&str> = defaults
        .iter()
        .map(|e| e.id.as_str())
        .filter(|id| !have.iter().any(|h| h == id))
        .collect();
    assert!(
        missing.contains(&"f5_tts"),
        "F5-TTS 应当在默认列表里（否则老配置永远看不到它）：{missing:?}"
    );

    // 每个引擎的 kind 都要能往返序列化 —— 加了变体忘记加 serde 属性时会在这里挂
    for e in &defaults {
        let wire = serde_json::to_string(&e.kind).unwrap();
        let back: anima_lib::tts::TtsEngineKind = serde_json::from_str(&wire)
            .unwrap_or_else(|err| panic!("引擎 {} 的 kind {wire} 无法回读：{err}", e.id));
        assert_eq!(
            serde_json::to_string(&back).unwrap(),
            wire,
            "引擎 {} 的 kind 往返不一致",
            e.id
        );
    }

    // 本地引擎必须有 base_url，否则连不上却只在第一句话时才报错
    for e in defaults
        .iter()
        .filter(|e| e.kind != anima_lib::tts::TtsEngineKind::Edge)
    {
        assert!(!e.base_url.is_empty(), "引擎 {} 缺 base_url", e.id);
    }
}

/// 两个本地服务的端口不能撞 —— 撞了的话后起来的那个静默失败。
#[test]
fn local_engines_use_distinct_ports() {
    let defaults = anima_lib::tts::TtsEngineConfig::defaults();
    let mut seen: Vec<(&str, &str)> = Vec::new();
    for e in &defaults {
        if e.base_url.is_empty() {
            continue;
        }
        if let Some((existing, _)) = seen.iter().find(|(_, url)| *url == e.base_url) {
            panic!(
                "引擎 {} 和 {existing} 的 base_url 撞了：{}",
                e.id, e.base_url
            );
        }
        seen.push((&e.id, &e.base_url));
    }
}
