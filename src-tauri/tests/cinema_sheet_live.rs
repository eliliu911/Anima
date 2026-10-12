//! 把生成的工作流真的提交给 ComfyUI。
//!
//! 单元测试只能证明「图是连通的」，证明不了 **ComfyUI 认这张图** ——
//! 节点名拼错、preset 名不对、输入字段少一个，都要等提交时才报错，
//! 而它报的错往往很难懂（"clip input is invalid: None"）。
//!
//! ```bash
//! cargo test --test cinema_sheet_live -- --ignored --nocapture
//! ```

use anima_lib::cinema_sheet as sheet;

const BASE: &str = "http://127.0.0.1:8188";

async fn submit(name: &str, wf: serde_json::Value) {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{BASE}/prompt"))
        .json(&serde_json::json!({ "prompt": wf, "client_id": "anima-test" }))
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await;
    let Ok(resp) = resp else {
        eprintln!("跳过 {name}：ComfyUI 没在跑");
        return;
    };
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "{name} 被 ComfyUI 拒绝：{}",
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );
    eprintln!(
        "  {name} ✓ 已排队 {}",
        body["prompt_id"].as_str().unwrap_or("?")
    );
}

#[tokio::test]
#[ignore = "需要本机 ComfyUI 在跑"]
async fn 三条管线的工作流都能被接受() {
    let ckpt = "RealVisXL_V5.0_fp16.safetensors";
    let lora = Some("CharTurnXLRetag_v4.safetensors".to_string());
    let re = Some("ref_yumiko.png".to_string());

    submit(
        "三视图(LoRA+参考图)",
        sheet::to_workflow(&sheet::turnaround(
            sheet::SheetEngine::Sdxl,
            ckpt,
            "1girl, short auburn hair",
            "black silk dress",
            lora.clone(),
            re.clone(),
            1,
            "t_turn",
        )),
    )
    .await;
    submit(
        "表情(仅参考图)",
        sheet::to_workflow(&sheet::expression(
            ckpt,
            "1girl, short auburn hair",
            "smiling warmly",
            re.clone(),
            2,
            "t_expr",
        )),
    )
    .await;
    submit(
        "动作(仅参考图)",
        sheet::to_workflow(&sheet::pose(
            ckpt,
            "1girl",
            "black silk dress",
            "running",
            re,
            None,
            3,
            "t_pose",
        )),
    )
    .await;
    submit(
        "最简(无 LoRA 无参考图)",
        sheet::to_workflow(&sheet::expression(ckpt, "1girl", "angry", None, 4, "t_min")),
    )
    .await;

    // 面部机位：另一个 LoRA，另一套提示词
    submit(
        "面部机位(FaceTurn LoRA)",
        sheet::to_workflow(&sheet::face_angles(
            ckpt,
            "1girl, short auburn hair",
            sheet::find_face_lora(),
            Some("ref_yumiko.png".into()),
            5,
            "t_face",
        )),
    )
    .await;

    // 带 ControlNet 的动作。**姿势参考图用现成的输出图** ——
    // 它只是喂给 OpenPose 预处理器提骨架，用什么图不重要，能提出骨架就行
    let mut with_pose = sheet::pose(
        ckpt,
        "1girl",
        "black dress",
        "running",
        Some("ref_yumiko.png".into()),
        None,
        6,
        "t_cn",
    );
    if let Some(cn) = sheet::find_controlnet() {
        with_pose.pose_control = Some(("ref_yumiko.png".into(), 0.8, cn));
        submit("动作(带 ControlNet 骨架)", sheet::to_workflow(&with_pose)).await;
    } else {
        eprintln!("  跳过 ControlNet：没找到模型");
    }
}
