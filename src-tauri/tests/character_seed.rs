//! 角色种子的存取。
//!
//! 种子存档解决的是**可复现**：不固定的话每次生成都是新的掷骰子，
//! 「上次那张好」找不回来，改了提示词也分不清差异是提示词还是运气。
//!
//! （跨镜头还是同一个人靠的是参考图 IPAdapter/PuLID，不是种子 ——
//! 两者解决的是不同问题。）

use anima_lib::cinema::CinemaCharacter;

fn sample() -> CinemaCharacter {
    CinemaCharacter {
        id: "c1".into(),
        name: "由美子".into(),
        role: "主角".into(),
        avatar_url: "avatar.png".into(),
        trigger_words: "1girl".into(),
        lora_model: None,
        lora_weight: 0.8,
        consistency_mode: "pulid".into(),
        clothing_prompt: Some("black dress".into()),
        multi_angles: vec![],
        asset_sheet: None,
        visual_dna: None,
        episode_id: None,
        seed: None,
    }
}

#[test]
fn 老档案没有seed字段也能读() {
    // **serde(default) 必须生效** —— 加字段前存的 characters.json
    // 里没有 seed，读不出来的话用户的角色会整个消失
    let old = r#"[{
        "id":"c1","name":"由美子","role":"主角","avatar_url":"a.png",
        "trigger_words":"1girl","lora_model":null,"lora_weight":0.8,
        "consistency_mode":"pulid","clothing_prompt":null
    }]"#;
    let list: Vec<CinemaCharacter> = serde_json::from_str(old).expect("老档案应该还能读");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].seed, None);
}

#[test]
fn 写种子不冲掉其它字段() {
    // 生成和编辑是两条独立的路。整份覆盖会把用户刚在界面上改的东西冲掉，
    // 而那种丢失没有任何提示
    let mut c = sample();
    c.clothing_prompt = Some("用户刚改的衣服".into());
    c.trigger_words = "用户刚改的触发词".into();

    let text = serde_json::to_string(&vec![c.clone()]).unwrap();
    let mut list: Vec<CinemaCharacter> = serde_json::from_str(&text).unwrap();
    list[0].seed = Some(42);

    assert_eq!(list[0].seed, Some(42));
    assert_eq!(list[0].clothing_prompt.as_deref(), Some("用户刚改的衣服"));
    assert_eq!(list[0].trigger_words, "用户刚改的触发词");
}

#[test]
fn 种子能被序列化回去() {
    let mut c = sample();
    c.seed = Some(123456);
    let text = serde_json::to_string(&c).unwrap();
    assert!(text.contains("\"seed\":123456"), "seed 没写进 JSON：{text}");
    let back: CinemaCharacter = serde_json::from_str(&text).unwrap();
    assert_eq!(back.seed, Some(123456));
}
