//! 真出一张分集封面看效果。**判定「做出来了」必须真的看图。**
//!
//! 默认 `#[ignore]` —— 它要出图（GPU）、要本机 ComfyUI 在跑。

#[tokio::test]
#[ignore = "要本机 ComfyUI，且会真的出一张图"]
async fn 出一张真封面() {
    let cfg = dirs::data_dir().unwrap().join("com.anima.desktop");
    let p = anima_lib::episode_cover::generate_episode_cover(&cfg, "drama_1789626704", "ep_01")
        .await
        .expect("封面没出来");
    println!("封面: {}", p.display());
    let img = image::open(&p).expect("读不了").to_rgba8();
    println!("尺寸: {}x{}", img.width(), img.height());
    assert!(img.width() > 0 && img.height() > 0);
}

/// **已经合成（甚至超分）好的成片，直接加片头** —— 不重渲染任何镜头。
///
/// 验三件：时长正好多出片头那几秒、按两次不会叠成两个片头、撤掉能回到原长。
#[tokio::test]
#[ignore = "要本机有合成好的 final.mp4"]
async fn 给现成的成片加片头() {
    let cfg = dirs::data_dir().unwrap().join("com.anima.desktop");
    let (proj, ep) = ("drama_1789626704", "ep_01");
    let ep_dir = cfg.join("projects").join(proj).join("episodes").join(ep);
    let secs = |p: &std::path::Path| -> f64 {
        let o = std::process::Command::new("ffprobe")
            .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
            .arg(p)
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).trim().parse().unwrap()
    };
    let final_mp4 = ep_dir.join("final.mp4");

    // 先确保是不带片头的状态
    anima_lib::episode_cover::set_cover_intro(&cfg, proj, ep, false).await.unwrap();
    let base = secs(&final_mp4);
    println!("原成片 {base:.2}s");

    let t = std::time::Instant::now();
    anima_lib::episode_cover::set_cover_intro(&cfg, proj, ep, true).await.unwrap();
    let once = secs(&final_mp4);
    println!("加片头后 {once:.2}s（用时 {:.1}s）", t.elapsed().as_secs_f32());
    assert!((once - base - anima_lib::episode_cover::COVER_INTRO_SECS).abs() < 0.6, "多出来的不是片头那几秒");
    assert!(anima_lib::episode_cover::has_cover_intro(&ep_dir));

    // 按第二次不能叠成两个片头
    anima_lib::episode_cover::set_cover_intro(&cfg, proj, ep, true).await.unwrap();
    let twice = secs(&final_mp4);
    println!("再按一次 {twice:.2}s");
    assert!((twice - once).abs() < 0.3, "按两次叠出了两个片头：{once:.2} → {twice:.2}");

    // 字幕要跟着让开
    let srt = anima_lib::cinema::export_srt(&cfg, proj, ep).unwrap();
    let first = srt.content.lines().find(|l| l.contains("-->")).unwrap().to_string();
    println!("第一条字幕 {first}");
    assert!(!first.starts_with("00:00:00,0"), "字幕没让开片头：{first}");

    // 撤掉回到原长
    anima_lib::episode_cover::set_cover_intro(&cfg, proj, ep, false).await.unwrap();
    assert!((secs(&final_mp4) - base).abs() < 0.2, "撤片头没回到原长");
    println!("撤掉后 {:.2}s ✓", secs(&final_mp4));
}
