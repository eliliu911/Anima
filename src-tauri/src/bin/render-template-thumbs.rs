//! 出构图模板的示例图。跑一次就够，出完**转成 jpg** 再提交进仓库。
//!
//! ```sh
//! for f in resources/template_thumbs/*.png; do
//!   ffmpeg -i "$f" -vf scale=768:-2 -q:v 4 "${f%.png}.jpg" && rm "$f"
//! done
//! ```
//!
//! ```sh
//! ./scripts/with-env.sh sh -c 'cd src-tauri && cargo run --bin render-template-thumbs'
//! ```
//!
//! **同一个角色、同一个 seed** —— 三张之间只有构图这一个变量，
//! 否则用户看到的差异里混着「换了个人」。
use anima_lib::agent_runtime::{RuntimeContext, RuntimePolicy, ToolCallRequest, ToolHandler};
use std::path::PathBuf;

const SUBJECT: &str =
    "林辰，二十八岁男性佣兵，短黑发，冷峻五官，黑色战术外套，颈侧有细纹身";

/// 示例图带风格出。**写实人像当示例难看**，而且会让人以为这套模板只能出写实。
/// 用 `painterly_3d_series` 预设的正向词（和风格页里那一档是同一份）。
const STYLE: &str = "painterly 3D animated series style, stylized realistic proportions, hand-painted texture overlays, brushstroke shading, dramatic rim light, expressive angular character design, cinematic color grading";

/// 和前端 `assetPrompt.ts` 的 `CHARACTER_TEMPLATES` **必须一致**。
/// 两处各写一份的话，示例图和实际出的图会长得不一样 ——
/// 而示例图的全部意义就是「点下去大概会得到什么」。
const TEMPLATES: &[(&str, &str, u32, u32)] = &[
    // 没有身体的角色：手机屏幕上的卡通 AI 头像、一团光、一个图标。
    // **方图** —— 它就是一个居中的图标，横幅会在两侧留一大片空。
    ("avatar",
     "Composition: a single centered avatar icon on a plain neutral background, front-facing, symmetrical, filling most of the frame. This subject has NO body: do not draw a torso, arms, legs, clothing, shoes or a standing pose. Do not draw a turnaround sheet, multiple views, panels or borders. Flat even lighting, clean silhouette, crisp edges, suitable for display on a screen.",
     1024, 1024),
    ("simple",
     "Composition: character reference sheet, single unified image, seamless layout without borders or frames, neutral gray background. Left half: large head close-up portrait (shoulders up, sharp facial details, front-facing, detailed skin texture). Right half: three equally-sized full-body standing poses arranged side by side (front view, side view, back view), head-to-toe fully visible, relaxed neutral pose. Consistent soft studio lighting across all views, no harsh shadows, even illumination.",
     1216, 832),
    ("detailed",
     "Composition: detailed character reference sheet, single unified image, seamless layout without borders or frames, neutral gray background. Left section: three full-body standing views side by side (front / side / back), head-to-toe visible, neutral relaxed pose. Upper right: large face close-up portrait (shoulders up, detailed skin texture, sharp eyes, pores visible). Lower right: three smaller head shots showing different angles (front, three-quarter, profile). Consistent soft studio lighting, no harsh shadows, even illumination across all panels.",
     1216, 832),
    // **这个 id 一度是 `design_sheet`，而前端那边早就改叫 `panel5` 了。**
    // 注释里写着「必须一致」，但没有任何东西在查 —— 于是它悄悄漂了，
    // 表现是那张卡永远显示「还没出示例图」。
    ("panel5",
     "Keep the same character identity across every panel. Composition: professional character design sheet, single unified image with dark neutral background (deep charcoal with a subtle gradient). Layout divided into labeled panels with thin border frames: - Top left: large dramatic character portrait (bust shot, three-quarter angle, moody rim lighting), unlabeled - Center: exactly three full-body standing views and no other figures in this row (front / side / back), labeled \"正面\" \"侧面\" \"背面\" - Top right: 4 expression close-ups in a row (calm, smiling, angry, surprised), labeled \"表情特写\" - Bottom left: 3-4 detail close-up panels showing the hands, the footwear, a collar and fastening detail, a belt or pocket detail, labeled \"细节特写\". Cinematic lighting, high detail, concept art quality, game character sheet aesthetic. Do not create an information table, stats card, UI panel, watermark, or extra character body.",
     1216, 832),
    // 中文四视图。**纯文生图，不用底图也不用 LoRA** —— 和下面那个
    // `quad`（Krea 2 + 版式 LoRA + 图生图）是两条路。
    ("quadzh",
     "高质量专业角色设定图，横向构图，纯白色纯净背景，中性摄影棚灯光，平光布光。布局结构（必须是角色四视图）：正面面部头部特写（占图片水平 1/3 的空间，**竖向放大，从头顶到胸口占满这一栏的完整高度**）＋ 全身正面视图、全身左侧面视图、全身背面视图（三个并列排列，占图片水平剩余的 2/3 的空间），四个视图之间用淡灰色(#E2E2E2)的 2px 细线分割。光影：柔和的前侧光，清晰的轮廓定义，自然的肤色，面部清晰、服装可辨识，平视镜头，完整全身，无裁剪。确保所有视图中的面部特征、发型、体型和服装保持一致。不得出现任何道具、武器、食物、饮料、手持物（角色必须空手）；不得出现复杂动作、情绪表情、面部遮挡；不得出现环境背景（仅纯白）；不得出现其他角色；不得出现文字、水印、标签、UI 元素；不得过度风格化。不得出现第二个头像或第二个身体。",
     1216, 832),
];

#[tokio::main]
async fn main() {
    // **落在项目根的 `resources/`，不是 `src-tauri/resources/`。**
    // `cargo run` 的工作目录是 `src-tauri/`，直接用相对路径会落错地方 ——
    // 而 `template_thumbs_dir()` 找的是项目根那个，于是图在磁盘上、
    // 界面上却是占位。（第一次就是这么落错的。）
    let out = anima_lib::cinema::workspace_root()
        .unwrap_or_else(|| PathBuf::from(".."))
        .join("resources")
        .join("template_thumbs");
    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!("建不了 {}：{e}", out.display());
        std::process::exit(1);
    }
    let work = std::env::temp_dir().join("anima-template-thumbs");
    let _ = std::fs::create_dir_all(&work);
    let ctx = RuntimeContext::new(work.clone(), RuntimePolicy::workspace_write(&work));
    let only: Vec<String> = std::env::args().skip(1).collect();

    for (id, composition, w, h) in TEMPLATES {
        if !only.is_empty() && !only.contains(&id.to_string()) {
            continue;
        }
        let target = out.join(format!("{id}.png"));
        // jpg 也算已有 —— 出完之后我们会转成 jpg 存进仓库
        // （三张 PNG 2.9 MB，转完 148 KB）。不认 jpg 的话每次都会重出。
        if target.is_file() || out.join(format!("{id}.jpg")).is_file() {
            println!("{id} 已有，跳过");
            continue;
        }
        let file = format!("tpl_{id}.png");
        let args = serde_json::json!({
            // **风格插在构图段之前。** 排在后面的话会被构图段那 600 字符的
            // 摄影语言淹没 —— 实测过（见 `finalPositive` 的注释）。
            "prompt": format!("{SUBJECT}，{STYLE}\n\n{composition}"),
            "engine": "zimage",
            "width": w, "height": h,
            // 三张同一个 seed —— 只有构图这一个变量
            "seed": 20_260_905u64,
            // **和资产出图走同一条路。** 不关的话，设置里那两个常驻
            // Z-Image LoRA 会挂上来 —— 示例图就不是「点下去会得到什么」了。
            "auto_loras": false,
            "output_filename": file,
        });
        print!("{id} … ");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let t = std::time::Instant::now();
        let r = anima_lib::agent_runtime::image_tool::GenerateImageTool
            .execute(&ctx, ToolCallRequest::new("generate_image", args))
            .await;
        match walk(&work, &file) {
            Some(p) => {
                let _ = std::fs::copy(&p, &target);
                println!("{:.0}s → {}", t.elapsed().as_secs_f32(), target.display());
            }
            None => println!("没出来：{:?}", r.status),
        }
    }
}

fn walk(dir: &PathBuf, name: &str) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(f) = walk(&p, name) {
                return Some(f);
            }
        } else if p.file_name().and_then(|s| s.to_str()) == Some(name) {
            return Some(p);
        }
    }
    None
}
