//! 出图质检：**生成完不是直接收下，而是先验一遍**。
//!
//! ## 为什么必须有这一层
//!
//! 同一段提示词，这次出 3 个人、下次出 4 个人 —— 扩散模型本来就是随机的。
//! 实测过：要「正面/侧面/背面三视图」，SDXL 给了 4 个人、而且全是正面。
//! 没有质检的话，这种废片会一路流进素材库，等到分镜时才发现角色对不上。
//!
//! 借鉴 wind-comic 的 Vision Retry：打分不过就换种子重生，而不是让人肉眼盯。
//!
//! ## 这里只做**便宜且确定**的检查
//!
//! 不上人脸检测器 —— 那要额外模型、额外依赖，而且它自己也会误判。
//! 这里只用几何和统计量，每一条都对应一种实际见过的废片：
//!
//! - 全黑：FLUX 在 MPS 上采样出 NaN 时就是整张全黑，而 ComfyUI 报「成功」
//! - 人数不对：三视图要 3 个人，多一个少一个都没法切
//! - 主体太小/太偏：构图跑掉，切出来是半截人

use image::{DynamicImage, GenericImageView};

/// 一张图里按列扫出来的「主体块」。
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub start: u32,
    pub end: u32,
}

impl Column {
    pub fn width(&self) -> u32 {
        self.end.saturating_sub(self.start)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Ok,
    /// 带上原因，直接给用户看 —— 「重生了 3 次」比「失败」有用得多
    Retry(String),
}

/// 判定一张图能不能用。`expect_subjects` = 期望的人数（三视图是 3，单格是 1）。
pub fn check(img: &DynamicImage, _expect_subjects: usize) -> Verdict {
    if is_blank(img) {
        return Verdict::Retry("整张图是空白/纯色 —— 采样很可能出了 NaN".into());
    }

    // **主体个数不做硬性拒绝。**
    //
    // 「一张图里有几个人」试过三种办法都不够稳：列方差阈值在背景占比不到
    // 一半时数出 0；周期性分割把单人像里抬起的手算成第二个人；
    // 贯通式背景竖条在背景有渐变时又找不到缝。
    //
    // 而它做的是**最激进的用法** —— 数不对就重试，三次都不对整格失败。
    // 实测后果：表情那一档六张全被拒，界面显示「表情模组 (0/6)」，
    // 日志里一行报错都没有。
    //
    // 误判的两个方向代价差得很远：放过一张多人图，用户重新生成一次就好；
    // 误拒一整档，用户看到的是空白，而且不知道为什么。所以这里只保留
    // 真正测得准的判据（空白图、主体覆盖率），个数留给调用方自己看。
    let (w, _) = img.dimensions();
    let covered: u32 = subject_columns(img).iter().map(|c| c.width()).sum();
    let ratio = covered as f32 / w as f32;
    // 密排图整排connected，覆盖率天然很高，这一条只对稀疏图有意义
    if !is_dense(img) && ratio < 0.15 {
        return Verdict::Retry(format!(
            "主体只占画面宽度的 {:.0}%，构图跑了",
            ratio * 100.0
        ));
    }
    Verdict::Ok
}

/// 判定一张图是否为纯色/近乎纯色的空白图（坏图）。
pub fn is_blank(img: &DynamicImage) -> bool {
    let small = img.thumbnail(64, 64).to_rgb8();
    let px: Vec<[u8; 3]> = small.pixels().map(|p| p.0).collect();
    if px.is_empty() {
        return true;
    }
    let n = px.len() as f32;
    let mean: [f32; 3] = (0..3)
        .map(|c| px.iter().map(|p| p[c] as f32).sum::<f32>() / n)
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    let var: f32 = px
        .iter()
        .map(|p| (0..3).map(|c| (p[c] as f32 - mean[c]).powi(2)).sum::<f32>())
        .sum::<f32>()
        / n;
    // 方差极低 = 纯色。阈值取得很松，只想抓「完全没画出东西」
    var < 12.0
}

/// 背景判据用**相对阈值**：以该图列方差的中位数当背景基准，超过 K 倍才算主体。
///
/// **绝对阈值在真实图上不成立。** 第一版写死 60，结果单人侧面图被数成 3 个 ——
/// 多出来的两块贴着画面左右边缘，那是背景渐变和暗角，不是人。
/// 而不同底图的背景「平整度」差很多：实测同一批图里，背景列方差的中位数
/// 从 221 到 860 不等，写死任何一个数都会在另一张图上翻车。
const COLUMN_VAR_K: f32 = 3.0;
/// 取中位数而不是更低的分位：分位取低了，画面左右边缘的暗角和背景渐变
/// 会被算成主体 —— 实测单人侧面图因此被数成 3 个（多出的两块正好贴着左右边框）。
/// 中位数成立的前提是**背景占据过半的列**，三视图和单人像都满足。
const BACKGROUND_PERCENTILE: f32 = 0.5;
/// 底线，防止纯色背景（分位数≈0）时阈值塌到 0 把噪点全算成主体
const COLUMN_VAR_FLOOR: f32 = 40.0;
/// 窄于这个比例的块当噪声丢掉（影子、边缘暗角）
const MIN_COLUMN_RATIO: f32 = 0.04;
/// 间隔小于这个比例就把两块并成一块（人物之间的手臂缝隙不该算两个人）
const MERGE_GAP_RATIO: f32 = 0.03;

/// 按列扫出主体块。**这是「数人数」的实现** ——
/// 影棚背景是干净平底，人物列的方差显著更高。
pub fn subject_columns(img: &DynamicImage) -> Vec<Column> {
    let small = img.thumbnail(384, 384).to_luma8();
    let (w, h) = (small.width(), small.height());
    if w == 0 || h == 0 {
        return Vec::new();
    }

    let mut vars = Vec::with_capacity(w as usize);
    for x in 0..w {
        let vals: Vec<f32> = (0..h).map(|y| small.get_pixel(x, y).0[0] as f32).collect();
        let mean = vals.iter().sum::<f32>() / h as f32;
        vars.push(vals.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / h as f32);
    }

    let mut sorted = vars.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((sorted.len() as f32 * BACKGROUND_PERCENTILE) as usize).min(sorted.len() - 1);
    let background = sorted[idx];
    let threshold = (background * COLUMN_VAR_K).max(COLUMN_VAR_FLOOR);
    let busy: Vec<bool> = vars.iter().map(|v| *v > threshold).collect();

    let mut blocks: Vec<Column> = Vec::new();
    let mut run: Option<u32> = None;
    for x in 0..w {
        match (busy[x as usize], run) {
            (true, None) => run = Some(x),
            (false, Some(s)) => {
                blocks.push(Column { start: s, end: x });
                run = None;
            }
            _ => {}
        }
    }
    if let Some(s) = run {
        blocks.push(Column { start: s, end: w });
    }

    // 合并靠得很近的块
    let gap = (w as f32 * MERGE_GAP_RATIO) as u32;
    let mut merged: Vec<Column> = Vec::new();
    for b in blocks {
        match merged.last_mut() {
            Some(last) if b.start.saturating_sub(last.end) <= gap => last.end = b.end,
            _ => merged.push(b),
        }
    }

    // 丢掉太窄的
    let min_w = (w as f32 * MIN_COLUMN_RATIO) as u32;
    let kept: Vec<Column> = merged.into_iter().filter(|c| c.width() > min_w).collect();

    // 换算回原图坐标
    let scale = img.width() as f32 / w as f32;
    kept.into_iter()
        .map(|c| Column {
            start: (c.start as f32 * scale) as u32,
            end: (c.end as f32 * scale) as u32,
        })
        .collect()
}

/// 数主体。**两种方法各管一种画面,按占比自动挑。**
///
/// 单靠一种都不行,实测过:
///
/// | 画面 | 阈值法 | 谷底法 |
/// |---|---|---|
/// | LoRA 六视图（肩并肩，几乎占满） | 0 ❌ | 6 ✓ |
/// | 四个人（之间有背景缝隙）        | 4 ✓ | 5 ❌ |
/// | 单人像                          | 1 ✓ | 3 ❌ |
///
/// 阈值法要求人物之间有背景缝隙;谷底法要求人物密排、边缘不被当成谷。
/// 所以先量「有多少列是忙的」:占满画面就走谷底,否则走阈值。
/// 这一格里有几个人。**单格 QC 用的就是它** —— 问的是
/// 「模型有没有多画出一个人」，不是「这一排该在哪儿切」。
///
/// ## 为什么不能用整排那套分割
///
/// 原先密排图会走 [`view_bounds`] 那条周期性分割，结果**单人图被数成
/// 2~4 个主体**：那套算法的前提是「画面里有一排等距的人」，
/// 而一张单人像里抬起的手、垂下的头发都能凑出一个"周期"。
///
/// 后果是表情那一档整批被 QC 拒掉（要 1 个、数出 2 个 → 重试 3 次 → 失败），
/// 界面上显示 `表情模组 (0/6)`，**日志里一行报错都没有**。
///
/// 所以这里只用「有背景缝隙才算两个人」的判据：一张图里的两个人之间
/// 几乎一定有背景，而一个人和自己的手之间没有。
pub fn count_subjects(img: &DynamicImage) -> usize {
    let strips = background_strips(img);
    // 缝隙个数 + 1 = 人数。左右两侧的空白不算缝隙（已在函数里排除）
    strips + 1
}

/// 画面中间有几条**贯通上下的纯背景竖条**。
///
/// ## 为什么用这个判据
///
/// 「这一格里有几个人」试过两种办法，都不行：
///
/// - **列方差阈值**（`subject_columns`）：背景占比不到一半时中位数落在主体上，
///   于是整张图都"不忙"，数出 0 个。实测三张竖构图人像全部返回 0。
/// - **周期性分割**（[`view_bounds`]）：前提是「一排等距的人」。
///   单人像里抬起的手、垂下的头发就能凑出一个"周期"，实测被数成 2~4 个。
///   后果是表情那一档整批被 QC 拒（要 1 个数出 2 个 → 重试 3 次 → 失败），
///   界面显示 `表情模组 (0/6)`，日志里一行报错都没有。
///
/// 贯通上下的背景竖条是唯一稳的信号：两个人并排一定隔着这样一条，
/// 而一个人和自己的手臂、头发之间不会 —— 那些只挡住画面的一小段高度。
fn background_strips(img: &DynamicImage) -> usize {
    let small = img.thumbnail(256, 256).to_rgb8();
    let (w, h) = (small.width(), small.height());
    if w < 16 || h < 16 {
        return 0;
    }
    // 背景取两侧边缘的中位色 —— 那儿几乎一定是背景。
    // **必须比 RGB，不能只比亮度**：影棚灰背景约 200，皮肤约 220，
    // 亮度只差 20，用亮度判会把整片皮肤当成背景（实测就是这么翻的）。
    // 皮肤是暖色（r 明显大于 b），灰背景是中性色，RGB 距离分得很开
    let mut edge: Vec<[u8; 3]> = Vec::new();
    for y in 0..h {
        for x in [0, 1, w - 2, w - 1] {
            edge.push(small.get_pixel(x, y).0);
        }
    }
    let med = |c: usize| {
        let mut v: Vec<u8> = edge.iter().map(|p| p[c]).collect();
        v.sort_unstable();
        v[v.len() / 2] as f32
    };
    let bg = [med(0), med(1), med(2)];
    let dist = |p: [u8; 3]| {
        (0..3)
            .map(|c| (p[c] as f32 - bg[c]).powi(2))
            .sum::<f32>()
            .sqrt()
    };

    // 一列有 97% 以上的像素接近背景色才算"空列"。
    // 留 3% 给影子和飘出来的发丝
    let empty: Vec<bool> = (0..w)
        .map(|x| {
            let near = (0..h)
                .filter(|&y| dist(small.get_pixel(x, y).0) < 30.0)
                .count();
            near as f32 / h as f32 > 0.97
        })
        .collect();

    // 只数**中间**的连续空列段。两侧的留白不是缝隙
    let first = (0..w as usize).find(|&x| !empty[x]);
    let last = (0..w as usize).rev().find(|&x| !empty[x]);
    let (Some(a), Some(b)) = (first, last) else {
        return 0;
    };

    // 缝隙至少要占画面宽度的 6%。
    // **这个下限刻意给得宽** —— 这个判据只用来做「明显有两个人」的硬拒绝，
    // 拿不准时宁可放过。放过一张多人图，用户重新生成一次就好；
    // 误拒一整档（重试 3 次全失败）用户看到的是「表情模组 0/6」，
    // 而且不知道为什么
    let min_gap = (w as f32 * 0.06).max(4.0) as usize;
    let mut gaps = 0;
    let mut run = 0usize;
    for x in a..=b {
        if empty[x] {
            run += 1;
        } else {
            if run >= min_gap {
                gaps += 1;
            }
            run = 0;
        }
    }
    gaps
}

/// 密排判据：列方差的 **中位数 / 90 分位**。
///
/// 密排图里中位数那一列几乎和最忙的列一样忙（到处都是人）；
/// 稀疏图的中位数是背景，和 p90 差着数量级。实测这个比值分得很干净：
///
/// ```text
/// LoRA 六视图（肩并肩） 0.566
/// 四个人（有缝隙）      0.171
/// 单人像                0.046 / 0.098
/// ```
///
/// 取 0.35 分界，两侧都留了两倍以上余量。
const DENSE_P50_P90: f32 = 0.35;

fn is_dense(img: &DynamicImage) -> bool {
    let mut v = busyness_profile(img);
    if v.is_empty() {
        return false;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q = |p: f32| v[((v.len() as f32 * p) as usize).min(v.len() - 1)];
    q(0.5) / q(0.9).max(1.0) >= DENSE_P50_P90
}

/// 谷底的位置（在 384 宽的缩略图坐标里）。
///
/// **切段必须用它，不能等分** —— 人物不是等间距的。等分切出来的每一段
/// 都跨着两个人，量出来的跨度就都差不多，尺度不一致这件事就测不出来了。
/// （我第一版就是等分，废图因此没被挡下来。）
pub fn valley_positions(img: &DynamicImage) -> Vec<usize> {
    let profile = busyness_profile(img);
    if profile.len() < 16 {
        return Vec::new();
    }
    let w = profile.len();
    let mean = profile.iter().sum::<f32>() / w as f32;
    if mean <= f32::EPSILON {
        return Vec::new();
    }

    // 平滑掉衣褶造成的单列抖动
    let smooth: Vec<f32> = (0..w)
        .map(|x| {
            let a = x.saturating_sub(2);
            let b = (x + 3).min(w);
            profile[a..b].iter().sum::<f32>() / (b - a) as f32
        })
        .collect();

    // ① 人物整体占的范围。两侧的空背景不参与找周期
    let floor = mean * 0.25;
    let lo = (0..w).find(|&x| smooth[x] > floor).unwrap_or(0);
    let hi = (0..w).rev().find(|&x| smooth[x] > floor).unwrap_or(w - 1);
    let span = hi.saturating_sub(lo);
    if span < w / 4 {
        return Vec::new();
    }

    // ② 自相关求周期。
    //
    // **三视图整排在构造上是周期性的** —— LoRA 排的是等间距的 N 个视角。
    // 逐个找谷底对不上，是因为相邻两个人的间隔深浅差很多：有的中间是
    // 干净背景（很深），有的是衣服贴着衣服（很浅）。深浅一刀切必然
    // 要么漏掉浅的、要么把衣褶算进来。周期不受这个影响 ——
    // 它是整条曲线一起投票投出来的。
    let score = |p: usize| -> f32 {
        if hi.saturating_sub(p) <= lo {
            return f32::MIN;
        }
        let n = hi - p - lo + 1;
        (lo..=hi - p)
            .map(|x| smooth[x] * smooth[x + p])
            .sum::<f32>()
            / n as f32
    };
    let cands: Vec<(usize, f32)> = (span / 8..=span / 2)
        .filter(|p| *p >= 8)
        .map(|p| (p, score(p)))
        .collect();
    if cands.len() < 5 {
        return Vec::new();
    }

    // 优先取曲线上的**局部峰** —— 一排等距的人会在周期处出现峰。
    //
    // 但**没有峰时不能放弃**：人物不严格等距时曲线会很平，
    // 实测一张三视图整排因此一个谷都找不到，分割结果是「1 个视角」，
    // 三视图那一档三次重试全被 QC 拒（「不够切三格」），整档失败。
    //
    // 找不到峰就退回整条曲线的最大值。误判的代价不对称：
    // 多切一刀顶多让某一格里挤着两个人（用户看得见、能重生成），
    // 少切到 0 刀直接让整档失败。
    //
    // 早先这里是硬性要求有峰，那是为了防「单人图被数成好几个人」——
    // 但那条路现在归 `count_subjects`/`background_strips` 管，
    // 不再走这里，所以这道闸没有必要了。
    let peaks: Vec<(usize, f32)> = (1..cands.len() - 1)
        .filter(|&i| cands[i].1 > cands[i - 1].1 && cands[i].1 >= cands[i + 1].1)
        .map(|i| cands[i])
        .collect();
    let peaks = if peaks.is_empty() {
        cands.clone()
    } else {
        peaks
    };
    let top = peaks.iter().map(|(_, s)| *s).fold(f32::MIN, f32::max);
    // 取达到峰值 95% 的**最小**周期 —— 自相关对倍频同样给高分
    // （一个周期里套两个人照样"对得上"），取 max 会把人数少算一半，
    // 实测六视图被切成三格
    let Some(&(period, _)) = peaks.iter().find(|(_, sc)| *sc >= top * 0.95) else {
        return Vec::new();
    };

    // ③ 每个理论分界附近，取**最深的那一列**做实际分界。
    // 人物不严格等间距，所以要在 ±period/3 的窗口里找，而不是直接用 k*period
    let count = ((span as f32 / period as f32).round() as usize).max(1);
    let mut cuts = Vec::new();
    for k in 1..count {
        let center = lo + k * span / count;
        let a = center.saturating_sub(period / 3).max(lo + 4);
        let b = (center + period / 3).min(hi.saturating_sub(4));
        if b <= a {
            continue;
        }
        let cut = (a..=b).min_by(|x, y| {
            smooth[*x]
                .partial_cmp(&smooth[*y])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if let Some(c) = cut {
            // 这一刀必须真的落在谷里 —— 否则是把一个人从中间劈开
            if smooth[c] < mean * MAX_CUT_LEVEL {
                cuts.push(c);
            }
        }
    }
    cuts.dedup();
    cuts
}

/// 分界那一列的忙碌度不能超过整幅均值的这个比例。
///
/// 挡的是「整排里其实只有 2 个人，却按周期硬切成 5 刀」——
/// 那些刀会落在人物身上，这里把它们丢掉。实测真实分界在 0.2~0.6 倍均值，
/// 落在人身上的假分界在 0.9 倍以上。
const MAX_CUT_LEVEL: f32 = 0.75;

/// 每一列的「忙碌度」（方差），缩放到统一宽度便于比较。
fn busyness_profile(img: &DynamicImage) -> Vec<f32> {
    let small = img.thumbnail(384, 384).to_luma8();
    let (w, h) = (small.width(), small.height());
    if w == 0 || h == 0 {
        return Vec::new();
    }
    (0..w)
        .map(|x| {
            let vals: Vec<f32> = (0..h).map(|y| small.get_pixel(x, y).0[0] as f32).collect();
            let mean = vals.iter().sum::<f32>() / h as f32;
            vals.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / h as f32
        })
        .collect()
}

/// 整排三视图的**尺度一致性**。
///
/// ## 为什么光数个数不够
///
/// 三视图整排的视角数不固定（实测 5~6 个），所以质检只能传「不限个数」，
/// 结果是一张「4 个人、全是正面、其中两个还是巨大半身」的废图照样通过 ——
/// 用户点了生成、系统报成功、切出来三格全是碎片。
///
/// ## 判据：每个人物的竖向跨度是否一致
///
/// 正经的 turnaround 里每个视角都顶天立地、尺度一致；跑偏时会混进半身特写，
/// 竖向跨度差一大截。实测（跨度极差 / 最大比最小）：
///
/// ```text
/// 废图（4 个人混着半身）  0.35 / 1.58
/// 好图（6 视角）          0.21 / 1.28
/// 好图（5 视角）          0.09 / 1.10
/// 好图（6 视角）          0.04 / 1.04
/// ```
///
/// 取 1.45 分界 —— 在最差的好图（1.28）和废图（1.58）之间，两侧都有余量。
const MAX_SPAN_RATIO: f32 = 1.45;

/// 整排是不是一个像样的 turnaround。
pub fn check_turnaround(img: &DynamicImage) -> Verdict {
    if is_blank(img) {
        return Verdict::Retry("整张图是空白/纯色 —— 采样很可能出了 NaN".into());
    }
    // **视角数用切段的结果，不再叫 count_subjects。**
    // 那个函数要在「阈值法/谷底法」之间猜画面是稀疏还是密排，猜错就数错；
    // 而这里已经按谷底把画面切开了，段数本身就是视角数 —— 少一次猜测，
    // 也少一处不一致（实测合成图上两者会给出不同答案）。
    let spans = figure_spans(img);
    if spans.len() < 3 {
        return Verdict::Retry(format!("只数出 {} 个视角，不够切三格", spans.len()));
    }
    let (min, max) = spans
        .iter()
        .fold((f32::MAX, 0.0f32), |(a, b), v| (a.min(*v), b.max(*v)));
    let ratio = max / min.max(1e-6);
    if ratio > MAX_SPAN_RATIO {
        return Verdict::Retry(format!(
            "各视角尺度差太多（最大/最小 {ratio:.2}）—— 多半混进了半身特写，切出来会是碎片"
        ));
    }
    Verdict::Ok
}

/// 按**真实的切片边界**量每段里主体的竖向跨度（占画面高度的比例）。
fn figure_spans(img: &DynamicImage) -> Vec<f32> {
    let bounds = view_bounds(img);
    if bounds.is_empty() {
        return Vec::new();
    }
    let rgb = img.to_rgb8();
    let h = rgb.height() as f32;
    let mut out = Vec::new();
    for &(x0, x1) in &bounds {
        let (sx0, sx1) = subject_extent(&rgb, x0, x1);
        if sx1 > sx0 {
            let top = subject_top(&rgb, sx0, sx1);
            let bottom = subject_bottom(&rgb, sx0, sx1);
            if bottom > top {
                out.push((bottom - top) as f32 / h.max(1.0));
            }
        }
    }
    out
}

/// 整排里每个视角在**原图**里的横向范围。
///
/// ## 为什么不能等分
///
/// 这个坑这个模块踩过两次。`figure_spans` 的注释里已经写了一遍
/// 「切段必须用谷底，不能等分 —— 人物不是等间距的」，
/// 但 `classify_views` 当时还是写成了 `w / count`，于是每一格都骑在
/// 两个人身上，量出来的肤色和躯干宽度是**两个视角的混合体**。
///
/// 实测后果：一张肉眼可见「六格、没有背面」的整排，
/// 被判成「五格，第三格是背面」—— 那一格其实是侧面。
/// 界面上「背面」那栏于是显示侧身，用户看到的就是「背面没生成」。
///
pub fn view_bounds(img: &DynamicImage) -> Vec<(u32, u32)> {
    let w = img.width();
    if w == 0 {
        return Vec::new();
    }
    // valley_positions 是在 384 宽的缩略图坐标里算的，换算回原图
    let scale = w as f32 / 384.0;
    let mut cuts: Vec<u32> = vec![0];
    cuts.extend(
        valley_positions(img)
            .into_iter()
            .map(|x| ((x as f32) * scale) as u32),
    );
    cuts.push(w);
    cuts.dedup();

    let raw: Vec<(u32, u32)> = cuts
        .windows(2)
        .map(|p| (p[0], p[1].min(w)))
        .filter(|(a, b)| b.saturating_sub(*a) > w / 20)
        .collect();

    let rgb = img.to_rgb8();
    let valid: Vec<(u32, u32)> = raw
        .into_iter()
        .filter(|&(x0, x1)| {
            let (sx0, sx1) = subject_extent(&rgb, x0, x1);
            // 必须检测到真实人物跨度，过滤两侧空墙
            sx1.saturating_sub(sx0) >= (w / 35).max(8)
        })
        .collect();

    if valid.is_empty() {
        vec![(0, w / 3), (w / 3, w * 2 / 3), (w * 2 / 3, w)]
    } else {
        valid
    }
}

/// 一格是哪个视角。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Front,
    Side,
    Back,
}

/// 头部肤色占比低于这个数就是**背面**。
///
/// 实测（6 张整排、逐格对着肉眼真值标）：
///
/// | 真值 | 肤色占比 |
/// |---|---|
/// | 背面 | 0.000 / 0.002 / 0.004 / 0.082 / 0.109 |
/// | 正面 | 0.130 ~ 0.196 |
/// | 侧面 | 0.123 ~ 0.282 |
///
/// **它分不出正面和侧面** —— 侧面的肤色反而更高（侧脸露出整个脸颊，
/// 而正脸有一半是五官的阴影和头发）。原先拿它当「正面判据」是搞反了。
/// 0.12 落在背面的上界（0.109）和正/侧的下界（0.123）之间。
const BACK_SKIN_MAX: f32 = 0.12;

/// 人物自身的宽高比，低于这个数就是侧身。
///
/// 实测（同一批图逐格标）：
///
/// | 真值 | 宽高比 |
/// |---|---|
/// | 侧面 | 0.105 ~ 0.161 |
/// | 正面 | 0.219 ~ 0.227 |
/// | 背面 | 0.196 ~ 0.261 |
///
/// 取 0.18，两侧相对间距都在 10% 以上。
///
/// **不用「相对本图最宽视角」那个量** —— 它在正/侧之间只有 0.65 vs 0.68
/// 的间距（4%），把图缩一半、重新分割一次就跨过去了，实测因此翻过。
/// 宽高比只依赖人物自己，稳得多。
const SIDE_ASPECT_MAX: f32 = 0.18;

/// 判断整排里每一格是正面 / 侧面 / 背面。
///
/// ## 为什么不能按位置猜
///
/// 我原先假设整排是「正面 → 逐步转身 → 背面」，于是取首/中/尾。
/// **LoRA 不保证这个顺序** —— 实测一张 5 视角的整排是
/// `正面 / 侧面 / 侧面 / 背面 / 正面`，取尾拿到的是正面，
/// 于是「背面全身三视图」那一格显示的是正脸。
///
/// ## 两个信号，缺一不可
///
/// - **头部肤色占比**：正面能看到整张脸，背面只有头发。
///   实测正面 19%、侧面 7.8%、背面 6.8% —— 分得出正面，
///   但**分不出侧面和背面**（都是 6~8%）。
/// - **躯干宽度**：侧身比正/背身窄。实测正面 100%、侧面 50~55%、背面 85%。
///
/// 合起来才够：肤色高 = 正面；否则看宽度，窄的是侧面、宽的是背面。
pub fn classify_views(img: &DynamicImage) -> Vec<View> {
    view_metrics(img)
        .into_iter()
        .map(|m| {
            // **判定顺序是量出来的，别调换。**
            // 先用肤色摘出背面（这是它唯一分得开的东西），
            // 剩下的用相对宽度分正/侧 —— 肤色在正侧之间完全不可用。
            if m.face_skin < BACK_SKIN_MAX {
                View::Back
            } else if m.aspect < SIDE_ASPECT_MAX {
                View::Side
            } else {
                View::Front
            }
        })
        .collect()
}

/// 一格的两个判据。抽出来是为了**阈值能拿实测值定**，而不是猜。
#[derive(Debug, Clone, Copy)]
pub struct ViewMetrics {
    /// 头部区域里的肤色占比。正面能看到整张脸，背面只有头发
    pub face_skin: f32,
    /// 人物宽度 / 这张图里最宽的那个视角。侧身明显窄
    pub rel_width: f32,
    /// 人物自身的宽高比。**不依赖同图里的其它格** ——
    /// 相对宽度在正/侧之间只有 0.03 的间距，缩放一下就跨过去了
    pub aspect: f32,
}

/// 逐格量出判据。**两条都必须相对本图，不能用绝对值** ——
/// 格宽由分割算法定、人物大小由构图定，都和"转没转身"无关。
pub fn view_metrics(img: &DynamicImage) -> Vec<ViewMetrics> {
    let bounds = view_bounds(img);
    if bounds.is_empty() {
        return Vec::new();
    }
    let rgb = img.to_rgb8();
    let h = rgb.height();

    let spans: Vec<(u32, u32)> = bounds
        .iter()
        .map(|&(x0, x1)| subject_extent(&rgb, x0, x1))
        .collect();
    let widest = spans
        .iter()
        .map(|(a, b)| b.saturating_sub(*a))
        .max()
        .unwrap_or(1)
        .max(1) as f32;

    spans
        .iter()
        .map(|&(sx0, sx1)| {
            if sx1 <= sx0 {
                return ViewMetrics {
                    face_skin: 0.0,
                    rel_width: 0.0,
                    aspect: 0.0,
                };
            }
            // 头部：从人物头顶往下一小段。
            // **不能带上肩膀** —— 露背礼服的裸背和肩膀也是肤色，
            // 带上之后背面的肤色占比和正面一样高
            let top = subject_top(&rgb, sx0, sx1);
            let (hy0, hy1) = (top, (top + (h as f32 * 0.13) as u32).min(h));
            let mut skin = 0u32;
            let mut total = 0u32;
            for y in hy0..hy1 {
                for x in sx0..sx1 {
                    let p = rgb.get_pixel(x, y).0;
                    let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
                    if r > 95 && g > 40 && b > 20 && (r - b) > 15 && r > g && g > b {
                        skin += 1;
                    }
                    total += 1;
                }
            }
            let bottom = subject_bottom(&rgb, sx0, sx1);
            let height = bottom.saturating_sub(top).max(1) as f32;
            ViewMetrics {
                face_skin: skin as f32 / total.max(1) as f32,
                rel_width: (sx1 - sx0) as f32 / widest,
                aspect: (sx1 - sx0) as f32 / height,
            }
        })
        .collect()
}

/// 人物头顶在第几行。**头部判据必须从这里量起** ——
/// 固定从画面顶端往下取一条，人物高矮不同时会带上肩膀。
fn subject_top(rgb: &image::RgbImage, x0: u32, x1: u32) -> u32 {
    let (w, h) = (rgb.width(), rgb.height());
    if x1 <= x0 || x1 > w {
        return 0;
    }
    let lum = |x: u32, y: u32| {
        let p = rgb.get_pixel(x, y).0;
        (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0
    };
    // 背景取最上面几行的中位数 —— 那儿几乎一定是空背景
    let mut bgv: Vec<f32> = Vec::new();
    for y in 0..(h / 40).max(2) {
        for x in x0..x1 {
            bgv.push(lum(x, y));
        }
    }
    bgv.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let bg = bgv[bgv.len() / 2];

    for y in 0..(h * 2 / 5) {
        let hits = (x0..x1).filter(|&x| (lum(x, y) - bg).abs() > 20.0).count();
        // 至少连着几列才算头，避免噪点
        if hits > ((x1 - x0) / 20).max(3) as usize {
            return y;
        }
    }
    0
}

/// 人物脚底在第几行。和 [`subject_top`] 一起给出人物自身的高度。
fn subject_bottom(rgb: &image::RgbImage, x0: u32, x1: u32) -> u32 {
    let (w, h) = (rgb.width(), rgb.height());
    if x1 <= x0 || x1 > w {
        return h;
    }
    let lum = |x: u32, y: u32| {
        let p = rgb.get_pixel(x, y).0;
        (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0
    };
    let mut bgv: Vec<f32> = Vec::new();
    for y in 0..(h / 40).max(2) {
        for x in x0..x1 {
            bgv.push(lum(x, y));
        }
    }
    bgv.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let bg = bgv[bgv.len() / 2];
    for y in (0..h).rev() {
        let hits = (x0..x1).filter(|&x| (lum(x, y) - bg).abs() > 20.0).count();
        if hits > ((x1 - x0) / 20).max(3) as usize {
            return y;
        }
    }
    h
}

/// 一格里人物占的横向范围。背景取该格的中位亮度 —— 影棚背景是平的。
pub fn subject_extent(rgb: &image::RgbImage, x0: u32, x1: u32) -> (u32, u32) {
    let h = rgb.height();
    let (ty0, ty1) = ((h as f32 * 0.20) as u32, (h as f32 * 0.60) as u32);
    if x1 <= x0 || ty1 <= ty0 {
        return (x0, x0);
    }
    let lum = |x: u32, y: u32| {
        let p = rgb.get_pixel(x, y).0;
        (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0
    };
    let mut all: Vec<f32> = Vec::new();
    for y in (ty0..ty1).step_by(4) {
        for x in x0..x1 {
            all.push(lum(x, y));
        }
    }
    if all.is_empty() {
        return (x0, x0);
    }
    all.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let bg = all[all.len() / 2];

    // **阈值要跟着这一格的对比度走。** 写死 18 灰阶时，黑裙配深灰背景
    // （SD1.5 那条路就是）整个人都算不进来，人物宽度量成 0
    let spread = (all[all.len() * 9 / 10] - all[all.len() / 10]).max(8.0);
    let thresh = (spread * 0.25).max(8.0);

    let mut lo = x1;
    let mut hi = x0;
    for x in x0..x1 {
        let diff: f32 = (ty0..ty1)
            .step_by(4)
            .map(|y| (lum(x, y) - bg).abs())
            .sum::<f32>()
            / ((ty1 - ty0) as f32 / 4.0).max(1.0);
        if diff > thresh {
            lo = lo.min(x);
            hi = hi.max(x);
        }
    }
    if hi <= lo {
        (x0, x0)
    } else {
        (lo, hi)
    }
}

/// 从整排里挑出正/侧/背三格的下标。
///
/// 挑不到某一类时回退到位置估计 —— 缺一格比整批失败好，
/// 但调用方要知道这是回退（返回值里那一格会重复）。
pub fn pick_views(img: &DynamicImage) -> Option<[usize; 3]> {
    let views = classify_views(img);
    let count = views.len();
    if count < 3 {
        return None;
    }
    let first_of = |want: View| views.iter().position(|v| *v == want);

    let front = first_of(View::Front).unwrap_or(0);
    // 背面从**后往前**找：整排常常是「正面 → 转身 → 背面 → 又转回正面」，
    // 从前往后找会拿到中间那个刚转过去的 3/4 背，不是最正的背面
    let back = views
        .iter()
        .rposition(|v| *v == View::Back)
        .unwrap_or(count - 1);
    // 侧面要避开已经选中的那两格 —— 否则「侧面」和「背面」会是同一张图，
    // 界面上看起来像少生成了一格。实测 pick 出 [0, 3, 3] 就是这么来的
    let side = (0..count)
        .find(|i| views[*i] == View::Side && *i != front && *i != back)
        .or_else(|| (0..count).find(|i| *i != front && *i != back))
        .unwrap_or(count / 2);

    Some([front, side, back])
}


// ---------------------------------------------------------------------------
// 跨格色温一致性
// ---------------------------------------------------------------------------

/// 一格的色温读数。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CellTemp {
    pub index: usize,
    pub name: String,
    /// R/B 通道均值比。**越大越暖。**
    pub ratio: f64,
}

/// 跨格色温核对的结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TempCheck {
    pub cells: Vec<CellTemp>,
    /// 离散度：`(max - min) / min`。
    pub spread: f64,
    /// 偏离中位数最远的格子。**这是给人看的线索，不是判决** ——
    /// 实测过四格全对而其中两格被点名的情况。
    pub outliers: Vec<usize>,
    /// 给人看的话。`None` = 没发现问题。
    pub note: Option<String>,
}

/// 离散到多少就把这个数摆出来让人看一眼。
///
/// # 它是「值得看一眼」的线，**不是「判错」的线**
///
/// 一开始我把它当判据用（超了就说「这几格可能不是同一个时刻」）。
/// 2026-09-11 端到端实跑两次把这个用法否掉了：
///
/// | 样本 | 离散 | 人眼判定 |
/// |---|---|---|
/// | 早先三张宫格 | 8.0% / 11.9% / 17.2% | 同一个环境 |
/// | 面馆四格 第一次 | 61.7% | 同一个环境（但厨房日光灯有一格没亮） |
/// | 面馆四格 第二次 | 70.0% | **同一个环境，四格都对** |
///
/// 第二次那组里，1/4 格画面中有一大片门外天光（冷），2/3 格是纯内景
/// （暖），于是全幅 R/B 干净地分成 1.4 和 2.3 两档 —— **差异来自取景，
/// 不是来自打光**。这正是 location-plate 列为合法差异的「朝窗与背窗」。
///
/// 试过把亮像素排掉（门外天光应该是亮的），**结果更糟**：71% → 91%。
/// 因为阴天的街面是中等亮度，排不掉；排掉的反而是室内的暖色高光。
///
/// 所以这个模块不再对色温下判断。它给数，人去看图。
/// 真正要核对的是灯具开关状态和主光位置，那两件事量不了。
pub const TEMP_SPREAD_REPORT: f64 = 0.25;

/// 单格偏离中位数多少算「差得最远的那一格」。**只用于点名，不是判错。**
pub const TEMP_OUTLIER_REPORT: f64 = 0.30;

/// **亮度和光比不做核对，这是故意的。**
///
/// 同一组样本里，中位亮度的离散是 16.5% ~ 72.0%，光比是 42.2% ~ 167.2% ——
/// 而这三组人眼都判「同一个环境」。原因是这种波动**本来就该有**：
/// 不同机位朝窗 / 背窗，亮度差几倍是正常的，location-plate 明确把
/// 「朝窗与背窗的自然亮度差」列为合法差异。
///
/// 卡它们只会把对的片子判成错的。色温不一样：换机位只改变我们从哪一侧
/// 看同一个光源，**不改变那个光源是暖是冷**。
///
/// 这个常量不参与任何计算，它存在只是为了让下一个想加亮度检查的人
/// 先读到这段。
pub const WHY_NOT_BRIGHTNESS: &str =
    "亮度和光比的大幅波动是合法的（朝窗/背窗），卡它们必然误报";

/// 一张图的色温读数：R 通道均值 / B 通道均值。
///
/// # 排掉过暗和过亮的像素
///
/// 近黑处 R 和 B 都是个位数，比值被噪声主导；吹掉的高光三通道都顶到 255，
/// 比值恒为 1，会把真实色温往中性拉。两头都留着的话，一张高对比度的图
/// 会显得比它实际更「中性」—— 而高对比度恰恰是宫格里最常见的情况。
///
/// 有效像素太少（整格几乎全黑或全白）时返回 `None`：这种格子没有色温
/// 可言，硬给一个数会让它变成假的异常格。
pub fn color_temp(img: &DynamicImage) -> Option<f64> {
    // 缩图再统计 —— 色温是整体量，全分辨率扫一遍纯属浪费。
    let small = img.thumbnail(96, 96).to_rgb8();
    let mut r_sum = 0f64;
    let mut b_sum = 0f64;
    let mut n = 0u32;
    for p in small.pixels() {
        let [r, g, b] = p.0;
        let luma = 0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64;
        if !(20.0..=245.0).contains(&luma) {
            continue;
        }
        r_sum += r as f64;
        b_sum += b as f64;
        n += 1;
    }
    // 不到一成像素可用就算读不出来。
    if n < (small.width() * small.height()) / 10 || b_sum <= 0.0 {
        return None;
    }
    Some(r_sum / b_sum)
}

/// 核对几张图是不是同一个时刻的同一个空间。
///
/// 输入是**已经切好的格子**（或者任何一组本该同源的图）。
///
/// # 只对场景宫格用，不对分镜宫格用
///
/// 场景宫格是「同一个空间的不同方向、**同一个时刻**」—— 色温本来就该一样。
/// 分镜宫格是「同一场戏的不同瞬间」，戏里从黄昏演到入夜是正常的，
/// 那时候色温**应该**变。在那条路上查这个是纯误报。
/// 名字只用于把问题说清楚，可以给空串。
///
/// 读不出色温的格子直接跳过，不当异常 —— 它们本来就没有色温可言。
pub fn check_color_temp(images: &[(String, DynamicImage)]) -> TempCheck {
    let cells: Vec<CellTemp> = images
        .iter()
        .enumerate()
        .filter_map(|(i, (name, img))| {
            color_temp(img).map(|ratio| CellTemp { index: i, name: name.clone(), ratio })
        })
        .collect();

    // 少于两格没有「一致性」可言。
    if cells.len() < 2 {
        return TempCheck { cells, spread: 0.0, outliers: Vec::new(), note: None };
    }

    let mut sorted: Vec<f64> = cells.iter().map(|c| c.ratio).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let min = sorted[0];
    let max = sorted[sorted.len() - 1];
    let median = sorted[sorted.len() / 2];
    let spread = if min > 0.0 { (max - min) / min } else { 0.0 };

    let outliers: Vec<usize> = cells
        .iter()
        .filter(|c| median > 0.0 && ((c.ratio - median) / median).abs() > TEMP_OUTLIER_REPORT)
        .map(|c| c.index)
        .collect();

    let note = if spread > TEMP_SPREAD_REPORT {
        let who = if outliers.is_empty() {
            String::new()
        } else {
            #[allow(clippy::let_and_return)]
            let names: Vec<String> = outliers
                .iter()
                .map(|i| {
                    cells
                        .iter()
                        .find(|c| c.index == *i)
                        .map(|c| {
                            if c.name.trim().is_empty() {
                                format!("第 {} 格", c.index + 1)
                            } else {
                                c.name.trim().to_string()
                            }
                        })
                        .unwrap_or_default()
                })
                .collect();
            format!("，差得最远的是 {}", names.join("、"))
        };
        Some(format!(
            "色温离散 {:.0}%{who}。**这不一定是错的** —— 实跑过同一个场景的\
             四个机位散到 70%，原因只是有的机位里有大片门外天光、有的是纯内景。\
             这个数不能判对错，只是提醒去看一眼：真正要核对的是\
             实际灯具的开关状态和主光位置，那两件事量不了。",
            spread * 100.0
        ))
    } else {
        None
    };

    TempCheck { cells, spread, outliers, note }
}

/// 从文件路径核对。读不出来的文件跳过 —— **这条路不能挡住出图**。
pub fn check_color_temp_paths(paths: &[std::path::PathBuf]) -> TempCheck {
    let loaded: Vec<(String, DynamicImage)> = paths
        .iter()
        .filter_map(|p| {
            let name = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            image::open(p).ok().map(|img| (name, img))
        })
        .collect();
    check_color_temp(&loaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    /// 造一张影棚图：平底 + 若干竖条主体。噪声是必须的 ——
    /// 纯色块方差为 0，测不出「主体比背景花」这个判据
    fn studio(w: u32, h: u32, subjects: &[(u32, u32)]) -> DynamicImage {
        let mut img = RgbImage::from_pixel(w, h, Rgb([235, 235, 235]));
        for (i, (x0, x1)) in subjects.iter().enumerate() {
            for x in *x0..*x1 {
                for y in (h / 8)..(h * 7 / 8) {
                    let n = ((x * 7 + y * 13 + i as u32 * 31) % 90) as u8;
                    img.put_pixel(x, y, Rgb([40 + n, 30 + n, 60 + n]));
                }
            }
        }
        DynamicImage::ImageRgb8(img)
    }

    /// 造整排：`heights` 是每个视角占画面高度的比例
    fn sheet(w: u32, h: u32, n: usize, heights: &[f32]) -> DynamicImage {
        let mut img = RgbImage::from_pixel(w, h, Rgb([235, 235, 235]));
        let cell = w / n as u32;
        for i in 0..n {
            let frac = heights[i % heights.len()];
            let top = ((1.0 - frac) * h as f32 / 2.0) as u32;
            let bottom = h - top;
            let x0 = i as u32 * cell + cell / 5;
            let x1 = (i as u32 + 1) * cell - cell / 5;
            for x in x0..x1 {
                for y in top..bottom {
                    let n2 = ((x * 7 + y * 13 + i as u32 * 31) % 90) as u8;
                    img.put_pixel(x, y, Rgb([40 + n2, 30 + n2, 60 + n2]));
                }
            }
        }
        DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn 尺度一致的整排放行() {
        let img = sheet(1536, 1024, 5, &[0.9]);
        assert_eq!(check_turnaround(&img), Verdict::Ok);
    }

    #[test]
    fn 混进半身特写的整排要重生() {
        // 这正是实测遇到的废图：两个全身 + 两个巨大半身，
        // 光数个数根本挡不住（个数是「不限」的）
        let img = sheet(1536, 1024, 4, &[0.95, 0.55, 0.55, 0.95]);
        match check_turnaround(&img) {
            Verdict::Retry(why) => assert!(why.contains("尺度"), "原因要说清是尺度问题：{why}"),
            v => panic!("混了半身应该判重生，实际 {v:?}"),
        }
    }

    #[test]
    fn 整排不够三个视角要重生() {
        let img = sheet(1024, 1024, 2, &[0.9]);
        assert!(matches!(check_turnaround(&img), Verdict::Retry(_)));
    }

    #[test]
    fn 整排空白要重生() {
        let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(512, 512, Rgb([0, 0, 0])));
        assert!(matches!(check_turnaround(&img), Verdict::Retry(_)));
    }

    #[test]
    fn 数得出三个主体() {
        let img = studio(1536, 1024, &[(200, 380), (690, 870), (1180, 1360)]);
        assert_eq!(subject_columns(&img).len(), 3);
        assert_eq!(check(&img, 3), Verdict::Ok);
    }

    /// **人数不对不再判重生。这是一个刻意的取舍，不是漏掉的。**
    ///
    /// 原先「要 3 个给了 4 个就重试」在合成图上一直是绿的，
    /// 而真实图上三种数法全都翻过：
    ///
    /// | 数法 | 在真实图上怎么坏的 |
    /// |---|---|
    /// | 列方差阈值 | 背景占比不到一半时中位数落在主体上，竖构图人像数出 0 |
    /// | 周期性分割 | 单人像里抬起的手凑出一个"周期"，数成 2~4 个 |
    /// | 贯通式背景竖条 | 背景有渐变时中间的背景列和边缘中位色差太多，缝找不到 |
    ///
    /// 而它做的是最激进的用法：数不对就重试，三次不对整格失败。
    /// 实测后果是表情那一档六张全被拒，界面显示「表情模组 (0/6)」，
    /// **日志里一行报错都没有**。
    ///
    /// 两个方向的误判代价差得很远 —— 放过一张多人图用户重生成一次就好，
    /// 误拒一整档用户看到的是空白且不知原因。所以只保留测得准的判据。
    /// 想重新开这道闸，先拿一批真实图证明数法稳了。
    #[test]
    fn 人数不对不再判重生_这是取舍() {
        let four = studio(
            1536,
            1024,
            &[(120, 280), (460, 620), (800, 960), (1140, 1300)],
        );
        assert_eq!(check(&four, 3), Verdict::Ok, "个数不该再挡图");
        // 真正测得准的两条仍然挡着
        let black = DynamicImage::ImageRgb8(RgbImage::from_pixel(512, 512, Rgb([0, 0, 0])));
        assert!(
            matches!(check(&black, 1), Verdict::Retry(_)),
            "全黑图必须挡住"
        );
    }

    #[test]
    fn 全黑要重生() {
        // FLUX 在 MPS 上采样出 NaN 就是这样，而 ComfyUI 报「成功」——
        // 不查的话这张会被当成正常素材存进库里
        let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(512, 512, Rgb([0, 0, 0])));
        assert!(matches!(check(&img, 1), Verdict::Retry(_)));
    }

    #[test]
    fn 纯白也要重生() {
        let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(512, 512, Rgb([255, 255, 255])));
        assert!(matches!(check(&img, 1), Verdict::Retry(_)));
    }

    #[test]
    fn 单格图数一个主体() {
        let img = studio(832, 1216, &[(300, 530)]);
        assert_eq!(check(&img, 1), Verdict::Ok);
    }

    #[test]
    fn 主体太小要重生() {
        let img = studio(1536, 1024, &[(700, 760)]);
        assert!(matches!(check(&img, 1), Verdict::Retry(_)));
    }

    #[test]
    fn 手臂缝隙不会被数成两个人() {
        // 同一个人身体和手臂之间有背景色缝隙，不合并的话会数成 2 个
        let img = studio(1536, 1024, &[(500, 700), (720, 780)]);
        assert_eq!(subject_columns(&img).len(), 1, "靠得近的块要合并");
    }


    // ---- 跨格色温一致性 ------------------------------------------------

    /// 造一张有色温的图：`warm` 越大越暖。带噪声，免得整张纯色。
    fn tinted(warm: f64) -> DynamicImage {
        let mut img = RgbImage::from_pixel(64, 64, Rgb([128, 128, 128]));
        for (x, y, p) in img.enumerate_pixels_mut() {
            let n = ((x * 7 + y * 13) % 40) as f64;
            let base = 90.0 + n;
            let r = (base * warm).clamp(0.0, 255.0) as u8;
            let b = (base / warm).clamp(0.0, 255.0) as u8;
            *p = Rgb([r, base as u8, b]);
        }
        DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn 暖图的色温比大于冷图() {
        let warm = color_temp(&tinted(1.3)).unwrap();
        let cool = color_temp(&tinted(0.8)).unwrap();
        assert!(warm > cool, "暖 {warm} 应该大于冷 {cool}");
        // 中性图应该接近 1
        let neutral = color_temp(&tinted(1.0)).unwrap();
        assert!((neutral - 1.0).abs() < 0.05, "中性图不该偏：{neutral}");
    }

    /// **全黑和全白读不出色温。**
    ///
    /// 近黑处 R 和 B 都是个位数，比值被噪声主导；吹掉的高光三通道都顶到
    /// 255，比值恒为 1。硬给一个数会让这种格子变成假的异常格。
    #[test]
    fn 全黑全白读不出色温() {
        let black = DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 32, Rgb([2, 2, 2])));
        let white = DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 32, Rgb([252, 252, 252])));
        assert!(color_temp(&black).is_none());
        assert!(color_temp(&white).is_none());
    }

    #[test]
    fn 同源的几格不报问题() {
        // 实测合格样本的离散是 8~17%，这里造 10% 左右
        let imgs: Vec<(String, DynamicImage)> = [1.00, 1.03, 1.05, 0.99]
            .iter()
            .enumerate()
            .map(|(i, w)| (format!("视角{i}"), tinted(*w)))
            .collect();
        let r = check_color_temp(&imgs);
        assert!(r.spread < TEMP_SPREAD_REPORT, "离散 {}", r.spread);
        assert!(r.note.is_none(), "{:?}", r.note);
        assert!(r.outliers.is_empty());
        assert_eq!(r.cells.len(), 4);
    }

    #[test]
    fn 一格换了主光就报出来() {
        let imgs: Vec<(String, DynamicImage)> = [
            ("入口正打", 1.0),
            ("柜台反打", 1.02),
            ("侧面", 0.99),
            ("走廊", 1.45), // 这一格把暖灯升成了主光
        ]
        .iter()
        .map(|(n, w)| (n.to_string(), tinted(*w)))
        .collect();
        let r = check_color_temp(&imgs);
        assert!(r.spread > TEMP_SPREAD_REPORT, "离散 {}", r.spread);
        let note = r.note.expect("该报没报");
        assert!(note.contains("走廊"), "没指出是哪一格：{note}");
        assert!(note.contains("差得最远的是"), "措辞要是「差得最远」不是「最不对」：{note}");
        assert_eq!(r.outliers, vec![3]);
    }

    /// **报出来的话必须说清它判不了对错。**
    ///
    /// 这条是端到端实跑逼出来的：面馆四格人眼看全对，色温离散 70% ——
    /// 因为两格里有大片门外天光、两格是纯内景。把这个数当判据用，
    /// 会把对的片子判成错的，而「报太多等于没报」。
    #[test]
    fn 报出来的话要说清它判不了对错() {
        let imgs: Vec<(String, DynamicImage)> =
            [1.0, 1.5].iter().map(|w| (String::new(), tinted(*w))).collect();
        let note = check_color_temp(&imgs).note.unwrap();
        assert!(note.contains("不一定是错的"), "{note}");
        assert!(note.contains("门外天光"), "要说清最常见的那个原因：{note}");
        assert!(!note.contains("可能不是同一个时刻"), "不能再下这个判断：{note}");
    }

    #[test]
    fn 没名字时用格号() {
        let imgs: Vec<(String, DynamicImage)> = [1.0, 1.0, 1.6]
            .iter()
            .map(|w| (String::new(), tinted(*w)))
            .collect();
        let note = check_color_temp(&imgs).note.unwrap();
        assert!(note.contains("第 3 格"), "{note}");
    }

    /// **少于两格没有一致性可言。** 一张图自己跟自己永远一致 ——
    /// 返回「通过」会让人以为验过了。
    #[test]
    fn 不够两格不做判断() {
        let one = vec![("只有一格".to_string(), tinted(1.0))];
        let r = check_color_temp(&one);
        assert_eq!(r.spread, 0.0);
        assert!(r.note.is_none());
        assert!(check_color_temp(&[]).cells.is_empty());
    }

    /// 读不出色温的格子跳过，**不当异常格**。
    #[test]
    fn 读不出色温的格子被跳过() {
        let imgs = vec![
            ("正常".to_string(), tinted(1.0)),
            ("全黑".to_string(), DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 32, Rgb([1, 1, 1])))),
            ("正常2".to_string(), tinted(1.03)),
        ];
        let r = check_color_temp(&imgs);
        assert_eq!(r.cells.len(), 2, "全黑那格不该有读数");
        assert!(r.note.is_none());
    }
}
