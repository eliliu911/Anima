//! 场景多视角联合图（宫格）。
//!
//! # 要解决的是场景一致性
//!
//! 同一个场景在不同镜头里长得不一样，根子和角色漂移一样：一句文字描述
//! 对应的是一大片可能性，每次采样落在哪儿看运气。
//!
//! 宫格给的是另一种约束：**一次采样内的 N 个格子天然一致** ——
//! 同一次去噪过程里画出来的九个视角，光线、材质、家具款式必然同源。
//! 这比任何提示词都硬，而且不用 3D。
//!
//! # 本机必须两段式，这一条和云端实现不一样
//!
//! moyin / DramaClaw 用的是云端模型（2K 起步），切完每格还够用。
//! 本机 Z-Image 出 1536×1024，切 3×3 每格只有 512×341 ——
//! 这正是设定卡那条已经吃过的亏（整排图的脸只有 40~46 px，提示词救不了）。
//!
//! 所以：
//!
//! ```text
//! 第一段：宫格出「布局 + 一致性」   低分辨率可以接受，要的是同一次采样
//! 第二段：逐格 i2i 放大精修        补细节，保持构图
//! ```
//!
//! [`PixelBudget`] 就是为了让第一段的产物**不被误当成成品**：
//! 每格低于门槛时明确说「这是布局稿，要精修」，而不是悄悄交一张糊的。
//!
//! # 视角从分镜反推，不是固定六宫格
//!
//! 抄 moyin：这一集只用到三个机位就出三个，不浪费像素预算。
//! 调用方给不出视角时才用默认那套。

/// 一个要出的视角。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewpoint {
    /// 格子里的标题，也是切出来之后的文件名依据。
    pub name: String,
    /// 这个视角看什么。空着就只发名字。
    pub description: String,
}

impl Viewpoint {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self { name: name.into(), description: description.into() }
    }
}

/// 调用方一个视角都给不出时用这套。
///
/// **不是「标准六视角」** —— 它只是个兜底。真实项目里视角应该从分镜反推，
/// 出的是这一集真正用得到的机位。
pub fn default_viewpoints() -> Vec<Viewpoint> {
    vec![
        Viewpoint::new("全景", "展示整个空间布局的宽角度"),
        Viewpoint::new("正打", "面向主要活动区的正面机位"),
        Viewpoint::new("反打", "与正打相对的机位，看见正打时背后的墙"),
        Viewpoint::new("入口", "从门口进来看到的第一眼"),
    ]
}

/// 宫格的行列。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    pub rows: u32,
    pub cols: u32,
}

impl Grid {
    pub fn cells(self) -> u32 {
        self.rows * self.cols
    }
}

/// 几个视角该排成什么宫格。
///
/// 抄 moyin 的判据：≤4 用 2×2，再多用 3×3，超过 9 个分页。
/// **不用 1×N 长条** —— 那是设定卡的形状，每格会被压得很扁。
pub fn grid_for(count: usize) -> Grid {
    match count {
        0 | 1 => Grid { rows: 1, cols: 1 },
        2 => Grid { rows: 1, cols: 2 },
        3 | 4 => Grid { rows: 2, cols: 2 },
        5 | 6 => Grid { rows: 2, cols: 3 },
        _ => Grid { rows: 3, cols: 3 },
    }
}

/// 视角太多时分页。每页最多一个 3×3。
pub fn paginate(viewpoints: &[Viewpoint]) -> Vec<Vec<Viewpoint>> {
    if viewpoints.is_empty() {
        return vec![];
    }
    viewpoints.chunks(9).map(<[Viewpoint]>::to_vec).collect()
}

/// 每格切出来有多大，够不够用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelBudget {
    pub cell_width: u32,
    pub cell_height: u32,
}

/// 低于这个短边就只能当布局稿。
///
/// **512 不是拍脑袋**：本机 Z-Image 出 1536×1024，3×3 每格 512×341 ——
/// 那是这条路的典型产物，能看清家具摆位和光线走向，但看不清材质。
/// 低于它（比如 4×4）连布局都读不准。
pub const LAYOUT_ONLY_BELOW: u32 = 512;

impl PixelBudget {
    pub fn of(sheet_width: u32, sheet_height: u32, grid: Grid) -> Self {
        Self {
            cell_width: sheet_width / grid.cols.max(1),
            cell_height: sheet_height / grid.rows.max(1),
        }
    }

    pub fn short_side(self) -> u32 {
        self.cell_width.min(self.cell_height)
    }

    /// 这一格能不能直接当成品用。
    ///
    /// **返回 false 不是错误**，是「还要走第二段精修」。
    /// 这个判据存在的意义是**别让布局稿被误当成品** ——
    /// 设定卡那次就是没人算这笔账，出来的脸只有 40~46 px。
    pub fn usable_as_final(self) -> bool {
        self.short_side() >= LAYOUT_ONLY_BELOW
    }

    /// 说给人听的一句话。
    pub fn note(self) -> String {
        if self.usable_as_final() {
            format!("每格 {}×{}，可直接使用", self.cell_width, self.cell_height)
        } else {
            format!(
                "每格只有 {}×{}（短边 {} < {}）—— 这是**布局稿**，\
                 家具摆位和光线走向可信，材质细节不可信。要当成品用得逐格 i2i 精修。",
                self.cell_width,
                self.cell_height,
                self.short_side(),
                LAYOUT_ONLY_BELOW
            )
        }
    }
}

/// 拼宫格的提示词。
///
/// **网格结构写在最前面**：出图模型对「画一张 N 行 M 列的网格」这件事
/// 要先建立整体版式，写在后面会被前面的场景描述带偏成一张普通图。
pub fn grid_prompt(scene_name: &str, scene_desc: &str, viewpoints: &[Viewpoint], grid: Grid) -> String {
    // **版式用英文的摄影术语起头，内容用中文。**
    //
    // `contact sheet` / `clean white borders separating panels` 是真实的
    // 摄影/美术词，训练语料里出现过 —— 比「一张精确的 N 行 M 列网格图」
    // 更容易命中版式。2026-09-11 五组对照实测：五种写法都能出对 3×3，
    // 但**英文版式措辞 + 逐格点名**那一组的视角区分度明显最好。
    //
    // 只用英文版式措辞（不逐格点名）会退化成九张几乎一样的图 ——
    // 「multiple camera angles」这种笼统说法模型不当回事。
    // **场景名不发给模型。** 2026-09-11 实测：提示词里同时写了
    // 「这个名字只用于理解，不要把它写进画面」「画面里不要出现任何文字：
    // 没有招牌…」，契约的负向清单里还有「任何招牌文字」—— 三道禁令，
    // 四格里仍然有两格把「安和面馆」当招牌画在了门楣上。
    //
    // 上一次「修」这个问题是把禁令拆成两句说清楚，那没用。根因是
    // **名字本身进了提示词**：它是一大段描述里唯一一串引号包着的中文，
    // 而 Z-Image 认得中文也画得出中文（那正是它被选为默认引擎的原因之一）。
    // 负向词对它又结构性无效（cfg=1）。
    //
    // 名字对画面没有任何贡献 —— 「安和面馆」长什么样全在 `scene_desc` 里。
    // 不发就不会被画出来。这是唯一真正生效的做法。
    let _ = scene_name;
    let mut lines = vec![format!(
        "a {}x{} grid contact sheet of environment concept art, {} cinematic thumbnails of \
         the SAME location, clean white borders separating panels, master visual study, \
         no people, no text.\n\
         这是同一个场景的 {} 个不同机位。",
        grid.cols,
        grid.rows,
        grid.cells(),
        grid.cells(),
    )];

    let desc = scene_desc.trim();
    if !desc.is_empty() {
        lines.push(desc.to_string());
    }

    // **默认空场。**
    //
    // 场景板的价值是「后续镜头共享同一可导航空间」—— 带上角色就绑死在
    // 某一场戏上，不能复用了。固定家具和功能设备要留，那是地理的一部分。
    //
    // 场景描述自己已经说了就不重复（空间契约的 `to_prompt_block` 带着
    // 它的空场政策）。实测撞过：同一句话在提示词里连着出现两遍。
    if !desc.contains("不要出现任何人物") {
        lines.push(
            // **一句「不要出现人物」不够，要把它会以什么形式溜进来都点名。**
            //
            // 抄的是 liblib 那份场景规范。他们把人物禁令展开成十来种形态，
            // 而我们只有一句笼统的 —— 而「剪影」「倒影」「影子」这三种
            // 最容易被当成氛围留下来，它们一样会让这张空场板绑死在某一场戏上。
            "画面里**不要出现任何人物**（这是空场景参考图，必须空无一人）：\
             不得出现人群、背影、剪影、人脸、手脚、人物倒影、人物影子、\
             照片里的人、屏幕里的人、镜中的人，也不得出现剧情事件和人物活动。\
             固定家具、灯具、设备等不可移动物要保留 —— 那是地理的一部分。"
                .into(),
        );
    }

    // **每格逐个点名。** 只说「不同视角」的话，模型会画出几个几乎一样的角度 ——
    // 那就白花了一次采样，而且不报错。
    //
    // 实测：四格里「全景」和「反打」出成了几乎同一个机位，只有墙上的字不同。
    // 光向和材质都对上了，**地理没对上** —— 柜台在第 2 格有、第 3 格同角度却没有。
    // 视角描述写成「面向柜台的正面机位」这种笼统说法不够，要写成
    // 「两两关系 + 视线终点」（从哪个入口看向哪里、尽头是什么），
    // 那是调用方的责任，这里只能把要求说清楚。
    for (i, v) in viewpoints.iter().enumerate() {
        let d = v.description.trim();
        let cell = if d.is_empty() {
            format!("第 {} 格：{}", i + 1, v.name.trim())
        } else {
            format!("第 {} 格：{} —— {}", i + 1, v.name.trim(), d)
        };
        lines.push(cell);
    }

    // 空格子要说清楚，否则模型会自己找东西填满
    let blanks = grid.cells() as usize - viewpoints.len().min(grid.cells() as usize);
    if blanks > 0 {
        lines.push(format!("剩下 {blanks} 个格子留空白，不要填充内容。"));
    }

    // **跨视角的一致性清单。**
    //
    // 这里逐项点名是有原因的：地理错位一眼能看出来，而**色温和主光位不一致
    // 常常各自都「很好看」**，只在两个视角被剪在一起时才暴露成两个不同的时间
    // 或两个不同的房间 —— 逐张单独看是抓不到的。
    //
    // 宫格这条路的价值正在这里：同一次采样里画出来的格子，这四项天然同源。
    // 但还是要说，因为不说的话模型可能把每格当成独立的图去画。
    lines.push(
        "每一格都要能看出**它和相邻格子的空间关系**：一格里看见的墙、门、\
         柜台，在别的格子里要能找到对应位置。反打格必须真的转到相对的一侧 ——\
         看见正打格里背对我们的那面墙，而不是把同一个机位挪一点。"
            .into(),
    );
    lines.push(
        "所有格子必须是**同一个空间、同一个时刻**，逐项对齐：\n         · 主光来源：同一个窗 / 同一组灯具，换机位只改变我们从哪一侧看它；\n         · 色温关系：冷暖主次一致，不能某一格把暖光升为主光；\n         · 光比与暗区方向：亮暗落差的量级、暗区落在空间的哪一侧；\n         · 实际灯具的开关状态：不能一格亮着吊灯、另一格熄着；\n         · 墙面 / 地面 / 天花的材质和颜色，家具款式和摆放位置；\n         · **建筑结构、空间比例、主体位置**：柱子几根、开间多宽、龙椅在哪，\n           换机位只改变我们从哪儿看，不改变它们本身。\n         **不得让四个视角变成四个不同的场景。**\n         逆光顺光的差别、遮挡造成的落影是正常的，那是同一光源换个方向看。"
            .into(),
    );
    // **两件事要分开说。**
    //
    // 实测撞过：只写「不要写文字标注」时，模型把它理解成「不要标注格子编号」，
    // 转头把场景名当成招牌画到了墙上（四格里有两格墙上写着「老茶馆」）。
    // 场景板是地理参考，画上招牌就把一个本来通用的空间钉死成了某一家店。
    // **畸变禁令。**
    //
    // 这一条是对着 liblib 的规范补的，而且我们比他们更需要它：
    // 方位词条里写死了 **24mm 广角**（`sceneViews.ts` 的 `lens: "f24"`）——
    // 不加这句，等于一边要广角一边不说别把画面拉变形。
    //
    // 「禁止景深太浅」同理：词条里 `depth: "deep"` 要的是深景深，
    // 而场景的构图段里原来写着 `depth of field`（浅景深的英文习惯说法），
    // 两句话在打架。
    lines.push(
        "镜头保持水平稳定：**不得倾斜、不得透视畸变、不得广角畸变、不得变形扭曲**，\
         不得出现鱼眼视角、斜角、极端俯视或极端仰视。\
         **景深要深**，前后都清楚，不要浅景深虚化。\
         正面那一格必须居中、对称、走中心线构图。"
            .into(),
    );

    lines.push(
        "格子之间用细白线分隔，不要画边框装饰、不要给格子加编号或标题。\
         **画面里不要出现任何文字**：没有招牌、没有匾额、没有墙上的字、\
         没有菜单价目，除非上面的视角描述里明确要求了某块标识。"
            .into(),
    );
    lines.join("\n")
}

/// 一个切出来的格子。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub index: usize,
    pub name: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// 按**实测的内容边界**算格子位置。
///
/// 和 [`cells_of`] 的区别只有一个：那个按均分算，这个按实际的内容段算 ——
/// 于是分隔线不会被切进格子里。格子不等宽时也对。
pub fn cells_from_bands(
    rows: &[(u32, u32)],
    cols: &[(u32, u32)],
    viewpoints: &[Viewpoint],
) -> Vec<Cell> {
    let mut out = Vec::new();
    for (r, &(y0, y1)) in rows.iter().enumerate() {
        for (c, &(x0, x1)) in cols.iter().enumerate() {
            let i = r * cols.len() + c;
            out.push(Cell {
                index: i,
                name: viewpoints
                    .get(i)
                    .map(|v| v.name.trim().to_string())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| format!("格{}", i + 1)),
                x: x0,
                y: y0,
                width: x1.saturating_sub(x0),
                height: y1.saturating_sub(y0),
            });
        }
    }
    out
}

/// 算出每个格子在联合图里的位置。
///
/// **按整数格宽切，余数留在最后一行/列** —— 不整除时如果按比例分，
/// 每格宽度会差一两个像素，拼回去对不上。
pub fn cells_of(sheet_width: u32, sheet_height: u32, grid: Grid, viewpoints: &[Viewpoint]) -> Vec<Cell> {
    let cw = sheet_width / grid.cols.max(1);
    let ch = sheet_height / grid.rows.max(1);
    // **切满实际的格子数，不按视角数截断。**
    //
    // 实测撞过：要 3×3 九格，模型画了 4×3 十二格，而按视角数只切出前 9 张 ——
    // 调用方一看「9 张对 9 个镜头」就放行了，实际上模型把内容铺在 12 格里，
    // 第 6 镜的画面落在了第 8 格。**每张单看都是合理的画面**，没人看得出来。
    //
    // 切满之后数量自然对不上，调用方的计数守卫才拦得住。
    (0..grid.cells() as usize)
        .map(|i| {
            let r = i as u32 / grid.cols;
            let c = i as u32 % grid.cols;
            Cell {
                index: i,
                // 多出来的格子给个能认的名字，不是空字符串
                name: viewpoints
                    .get(i)
                    .map(|v| v.name.trim().to_string())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| format!("格{}", i + 1)),
                x: c * cw,
                y: r * ch,
                width: cw,
                height: ch,
            }
        })
        .collect()
}

/// 从工具参数里读视角表。
///
/// 三种写法都认：字符串数组（只有名字）、对象数组（名字 + 描述）、
/// 以及完全没给（用 [`default_viewpoints`]）。
///
/// **名字要有语义。** 叫「视角1」「参考」的名字下游没法按名索引 ——
/// 切出来的文件名、以后 `@` 引用它的时候，靠的都是这个名字。
pub fn viewpoints_from_args(v: Option<&serde_json::Value>) -> Vec<Viewpoint> {
    let Some(arr) = v.and_then(|x| x.as_array()) else {
        return default_viewpoints();
    };
    let out: Vec<Viewpoint> = arr
        .iter()
        .filter_map(|item| match item {
            serde_json::Value::String(s) if !s.trim().is_empty() => {
                Some(Viewpoint::new(s.trim(), ""))
            }
            serde_json::Value::Object(_) => {
                let name = item.get("name").and_then(|x| x.as_str()).unwrap_or("").trim();
                if name.is_empty() {
                    return None;
                }
                let desc = item
                    .get("description")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .trim();
                Some(Viewpoint::new(name, desc))
            }
            _ => None,
        })
        .collect();
    if out.is_empty() { default_viewpoints() } else { out }
}

/// 逐格精修的参数。
///
/// # 为什么宫格必须有第二段
///
/// 第一段的产物是**布局稿**：本机 3×3 每格 512×341，能定构图、景别、
/// 站位和光线走向，定不了材质和脸。要当成品用，得在它基础上放大精修。
///
/// 两段式不是我们发明的 —— DramaClaw 的宫格之后也接 i2i 高清修复，
/// 只是它第一段的起点（云端 2K+）比我们高。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RefinePlan {
    pub width: u32,
    pub height: u32,
    pub denoise: f64,
}

/// 重绘强度的上限。**超过这个数构图就保不住了。**
///
/// i2i 的 denoise 是「从第几步开始重画」：0 完全不动，1 等于重新生成。
/// 精修要的是「保持构图、补细节」，实测经验和各家默认都落在 0.3~0.45。
/// 项目里 i2i 的默认值是 0.65 —— 那是**修图**的场景（用户想让它改），
/// 精修不能用那个数：构图正是第一段唯一的产出，重画掉就白花了那次采样。
pub const MAX_REFINE_DENOISE: f64 = 0.45;

/// 放大倍数上限。
///
/// i2i 补的是细节，不是分辨率 —— 放大超过 2 倍，模型没有足够的像素依据，
/// 出来的是「涂抹出来的细节」而不是原构图的细节，还容易改掉物体形状。
/// 要更大就分两次。
pub const MAX_UPSCALE: f64 = 2.0;

/// 算出这一格该精修到多大、重绘多少。
///
/// - **保持宽高比**：改了比例，构图就变了，而构图是第一段唯一的产出。
/// - **32 对齐**：引擎的硬约束（H3 直接拒绝非 32 倍数，SDXL 会静默出错位图）。
/// - 放大倍数和重绘强度都**夹紧**，见 [`MAX_UPSCALE`] 和 [`MAX_REFINE_DENOISE`]。
pub fn refine_plan(cell_w: u32, cell_h: u32, target_long_side: u32, denoise: f64) -> RefinePlan {
    let (cw, ch) = (cell_w.max(1), cell_h.max(1));
    let long = cw.max(ch) as f64;
    let want = target_long_side.max(1) as f64;
    let factor = (want / long).clamp(1.0, MAX_UPSCALE);
    let align = |v: f64| ((v / 32.0).round().max(1.0) * 32.0) as u32;
    RefinePlan {
        width: align(cw as f64 * factor),
        height: align(ch as f64 * factor),
        denoise: denoise.clamp(0.05, MAX_REFINE_DENOISE),
    }
}

/// 精修这一格用的提示词。
///
/// **重复第一段的画风和这一格的内容，不要加新东西。**
/// 精修阶段加新元素会让它和别的格子分家 —— 而九格一致正是第一段的产出，
/// 第二段的职责是**保住它**，不是改进它。
pub fn refine_prompt(style: &str, cell_desc: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let s = style.trim();
    let d = cell_desc.trim();
    if !d.is_empty() {
        parts.push(d);
    }
    if !s.is_empty() {
        parts.push(s);
    }
    if parts.is_empty() {
        return "保持原构图，补足材质和细节，不要改变物体位置和形状".into();
    }
    format!(
        "{}。保持原构图和物体位置不变，只补足材质、光影和细节",
        parts.join("，")
    )
}

/// 从图里**实测**格子布局。
///
/// # 为什么非要实测
///
/// 提示词说「2 行 2 列」，模型不一定听。实测撞过：要 4 格出了 6 格（3×2）——
/// 而切分器照样「成功」切了 4 块，**每块横跨两个格子**，不报错。
/// 切出来的是垃圾，但文件都在、数量也对。
///
/// 判据是分隔线：宫格之间是接近纯白的整行/整列。找不到分隔线就返回 `None`
/// （模型可能画成了一张普通的房间图），**那时候宁可不切**。
pub fn detect_grid(img: &image::DynamicImage) -> Option<Grid> {
    bands_of(img).map(|(rows, cols)| Grid {
        rows: rows.len() as u32,
        cols: cols.len() as u32,
    })
}

/// 实测每一行/列**内容**的起止（不含分隔线）。
///
/// # 为什么不能按「宽 ÷ 列数」均分
///
/// 均分会把分隔线也切进格子里 —— 每张切出来的图四周带一圈白边。
/// 那圈白边会一路进精修、进参考图、进首帧，最后变成画面里的一道白框，
/// **而图本身「看起来是好的」**。实测撞到了：精修出来的图四周有白框。
///
/// 顺便还解决了格子不等宽的情况（模型不保证均分）。
#[allow(clippy::type_complexity)]
pub fn bands_of(img: &image::DynamicImage) -> Option<(Vec<(u32, u32)>, Vec<(u32, u32)>)> {
    use image::GenericImageView;
    let (w, h) = img.dimensions();
    if w < 32 || h < 32 {
        return None;
    }
    let gray = img.to_luma8();
    let is_light_row = |y: u32| (0..w).all(|x| gray.get_pixel(x, y).0[0] >= 235);
    let is_light_col = |x: u32| (0..h).all(|y| gray.get_pixel(x, y).0[0] >= 235);

    // 数中间的内容段，就是行列数。
    //
    // **太薄的段不算一格。** 实测撞过：外边框右下角差 1 像素没铺满，
    // 剩下那 1 行被数成了第三行，于是 2×2 认成了 3×3 —— 而按 3×3 切
    // 会切出 9 块横跨格子的碎片，不报错。
    // 一格至少要占到这一维的 5%，否则它是边框的毛边，不是格子。
    let bands = |n: u32, light: &dyn Fn(u32) -> bool| -> Vec<(u32, u32)> {
        let min_cell = (n / 20).max(8);
        let mut out: Vec<(u32, u32)> = Vec::new();
        let mut start: Option<u32> = None;
        for i in 0..n {
            if light(i) {
                if let Some(st) = start.take() {
                    if i - st >= min_cell {
                        out.push((st, i));
                    }
                }
            } else if start.is_none() {
                start = Some(i);
            }
        }
        if let Some(st) = start {
            if n - st >= min_cell {
                out.push((st, n));
            }
        }
        out
    };
    // **严格判据（整行全亮）找出来的内容段还带着分隔线的毛边。**
    //
    // 真实分隔线有抗锯齿：边上那几列「几乎全白但不全白」，
    // 严格判据不认它们，于是它们留在了内容里 —— 切出来还是有白条。
    // 实测：按严格判据切，766 宽的格子左边仍有白边。
    //
    // 所以再向内修一遍：只要这一行/列**大部分**是亮的就继续往里走。
    let mostly_light_row = |y: u32| {
        (0..w).filter(|&x| gray.get_pixel(x, y).0[0] >= 235).count() * 10 >= (w as usize) * 9
    };
    let mostly_light_col = |x: u32| {
        (0..h).filter(|&y| gray.get_pixel(x, y).0[0] >= 235).count() * 10 >= (h as usize) * 9
    };
    let trim = |band: (u32, u32), mostly: &dyn Fn(u32) -> bool| -> (u32, u32) {
        let (mut a, mut b) = band;
        // 最多修掉这一段的 15% —— 再多就不是毛边，是把内容啃掉了
        let cap = ((b - a) / 7).max(1);
        let (lo, hi) = (a + cap, b.saturating_sub(cap));
        while a < lo && mostly(a) {
            a += 1;
        }
        while b > hi && mostly(b - 1) {
            b -= 1;
        }
        (a, b)
    };

    let rows: Vec<(u32, u32)> = bands(h, &is_light_row)
        .into_iter()
        .map(|band| trim(band, &mostly_light_row))
        .collect();
    let cols: Vec<(u32, u32)> = bands(w, &is_light_col)
        .into_iter()
        .map(|band| trim(band, &mostly_light_col))
        .collect();
    // 1×1 说明压根没有分隔线 —— 那不是宫格
    if rows.is_empty() || cols.is_empty() || (rows.len() == 1 && cols.len() == 1) {
        return None;
    }
    // 段数多到离谱说明认错了（比如画面本身有大片高光）
    if rows.len() > 6 || cols.len() > 6 {
        return None;
    }
    Some((rows, cols))
}

/// 把联合图切成一张张单图。
///
/// 落在**联合图同目录**下，命名 `<联合图名>_<序号>_<视角名>.png` ——
/// 序号在前是为了排序稳定，视角名在后是为了人能认出来。
///
/// # 失败只影响那一格
///
/// 某一格写盘失败时跳过它继续切下一格，最后把成功的那些报上去。
/// 让九格里坏一格就整个失败，是把一个部分可用的结果变成完全没有。
pub fn split_sheet(
    sheet: &std::path::Path,
    grid: Grid,
    viewpoints: &[Viewpoint],
) -> Result<Vec<std::path::PathBuf>, String> {
    let img = image::open(sheet).map_err(|e| format!("读不出联合图 {}：{e}", sheet.display()))?;
    let (w, h) = (img.width(), img.height());

    // **1×1 不是宫格**，整张就是那一格，不用检测也不该检测
    // （一张普通的图本来就没有分隔线）。
    // **按实测的内容边界切，不按均分。**
    //
    // 均分会把分隔线切进格子里 —— 每张图四周带一圈白边，而它会一路进
    // 精修、进参考图、进首帧，最后变成画面里的一道白框。实测撞到了。
    let bands = if grid.cells() > 1 { bands_of(&img) } else { None };

    let grid = if grid.cells() <= 1 {
        grid
    } else {
    // **按图里实际画成什么样切，不按我们要求的样子切。**
    //
    // 模型不一定听行列数。实测撞过：要 2×2 出了 3×2，而按 2×2 切照样
    // 「成功」—— 每块横跨两个格子，文件都在、数量也对，就是内容全错。
    match detect_grid(&img) {
        Some(actual) if actual == grid => grid,
        Some(actual) => {
            tracing::warn!(
                "联合图实际是 {}×{}，要求的是 {}×{} —— 按实际的切",
                actual.rows, actual.cols, grid.rows, grid.cols
            );
            actual
        }
        None => {
            return Err(format!(
                "这张图里找不到宫格分隔线（{w}×{h}）—— 模型可能画成了一张普通的图。\
                 没有按 {}×{} 硬切，因为那样会切出一堆横跨格子的碎片，而且不报错。",
                grid.rows, grid.cols
            ));
        }
    }
    };
    let dir = sheet.parent().unwrap_or(std::path::Path::new("."));
    let stem = sheet.file_stem().and_then(|s| s.to_str()).unwrap_or("sheet");

    // 有实测边界就用它，没有（1×1）退回均分
    let cells = match &bands {
        Some((rows, cols)) => cells_from_bands(rows, cols, viewpoints),
        None => cells_of(w, h, grid, viewpoints),
    };

    let mut out = Vec::new();
    for cell in cells {
        // 越界的格子直接跳过 —— 图比预期小时（引擎没按要求出尺寸）
        // 裁下去会 panic，而那只是这一格没有，不该炸掉整次调用
        if cell.x + cell.width > w || cell.y + cell.height > h {
            tracing::warn!(
                "联合图实际尺寸 {w}×{h} 装不下第 {} 格，跳过",
                cell.index + 1
            );
            continue;
        }
        let safe: String = cell
            .name
            .chars()
            .filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
            .collect();
        let path = dir.join(format!("{stem}_{}_{}.png", cell.index + 1, safe.trim()));
        let sub = img.crop_imm(cell.x, cell.y, cell.width, cell.height);
        match sub.save(&path) {
            Ok(()) => out.push(path),
            Err(e) => tracing::warn!("第 {} 格写盘失败：{e}", cell.index + 1),
        }
    }
    if out.is_empty() {
        return Err("联合图一格都没切出来".into());
    }
    Ok(out)
}

/// 打真实 ComfyUI 的验证。**默认 `#[ignore]`**。
///
/// ```bash
/// cd src-tauri && cargo test --lib scene_sheet_live -- --ignored --nocapture
/// ```
///
/// **为什么非有这一层**：版式数学和切分都能单测，但「宫格提示词发出去，
/// 模型到底画不画成网格」是模型侧的事 —— 它可能画成一张普通的房间图，
/// 而那时候切分会照常成功，切出九块毫无意义的碎片，**不报错**。
#[cfg(test)]
mod live {
    use super::*;

    /// 拿**真实出的那两张图**验守卫。不需要 ComfyUI，只要文件在。
    #[test]
    #[ignore = "要先跑过 scene_sheet_live"]
    fn detect_grid_认得出真实出图的布局() {
        for (name, want) in [
            ("/tmp/anima-scene-sheet/sheet.png", Grid { rows: 2, cols: 2 }),
            // **这一张是实测里模型没听话的那次**：要 2×2，出了 3×2
            ("/tmp/anima-scene-sheet/sheet2.png", Grid { rows: 2, cols: 3 }),
        ] {
            let p = std::path::Path::new(name);
            if !p.is_file() {
                println!("跳过（还没出过图）：{name}");
                continue;
            }
            let img = image::open(p).unwrap();
            let got = detect_grid(&img);
            println!("{name}: 实测 {got:?}，预期 {want:?}");
            assert_eq!(got, Some(want), "{name} 的布局认错了");
        }
    }

    /// 拿模型没听话那张（要 2×2 出了 3×2）真切一次，验它按**实际**布局切。
    #[test]
    #[ignore = "要先跑过 scene_sheet_live"]
    fn 按实际布局切模型没听话的那张() {
        let sheet = std::path::Path::new("/tmp/anima-scene-sheet/sheet2.png");
        if !sheet.is_file() {
            println!("跳过（还没出过图）");
            return;
        }
        let views = vec![
            Viewpoint::new("全景", ""), Viewpoint::new("正打", ""),
            Viewpoint::new("反打", ""), Viewpoint::new("窗边", ""),
            Viewpoint::new("第五格", ""), Viewpoint::new("第六格", ""),
        ];
        // 传进去的是我们**要求**的 2×2，实际图是 3×2
        let cells = split_sheet(sheet, Grid { rows: 2, cols: 2 }, &views).unwrap();
        for c in &cells {
            let d = image::open(c).unwrap();
            println!("  {} → {}×{}", c.display(), d.width(), d.height());
        }
        // 3×2 = 6 格，不是按 2×2 切出的 4 格
        assert_eq!(cells.len(), 6, "该按实际的 3×2 切");
        let first = image::open(&cells[0]).unwrap();
        assert!(first.width() < 550, "每格该是 512 宽左右，不是 768：{}", first.width());
    }

    /// **对照实验：什么样的提示词能让模型真的画成 N×M。**
    ///
    /// 这是这条路目前唯一的阻碍 —— 两次实跑，要 2×2 出了 3×2、
    /// 要 3×3 出了 4×3。切分和守卫能接住（按实测布局切、对不上就拒绝分配），
    /// 但「拒绝分配」意味着经常走不通。
    ///
    /// 判据是 `detect_grid` 认出来的行列数 == 要求的行列数。**不看好不好看。**
    #[tokio::test]
    #[ignore = "要本机 ComfyUI"]
    async fn scene_sheet_live_宫格版式对照() {
        let want = Grid { rows: 3, cols: 3 };
        let topic = "民国风格的老茶馆，木梁、青砖地、临街一排木格窗，午后斜光";

        // **视角要给真实描述。**
        //
        // 第一版这里用的是 `视角1`..`视角9` + 空描述 —— 提示词 literally 写成
        // 「第 1 格：视角1」，不含任何信息，于是九格出成了几乎一样的图。
        // 那是我测试自己的问题，不是提示词写法的问题；
        // 但它正好示范了「无语义命名，下游无法按名索引」这条反模式。
        let views = vec![
            Viewpoint::new("入口全景", "站在临街木门内侧朝店堂深处看，尽头是靠后墙的柜台"),
            Viewpoint::new("柜台正打", "从店堂中央看向后墙柜台，柜台占画面中心"),
            Viewpoint::new("柜台反打", "站在柜台后面朝门口看，尽头是那扇临街木门"),
            Viewpoint::new("窗边", "贴着临街木格窗往里看，窗在画面左侧，桌椅沿窗排开"),
            Viewpoint::new("梁下仰视", "从桌边仰头看木梁和吊灯"),
            Viewpoint::new("地面俯视", "俯视青砖地和桌腿投下的光斑"),
            Viewpoint::new("角落", "店堂最里侧的墙角，堆着茶罐和竹凳"),
            Viewpoint::new("门外回望", "从门槛外往里看，木门半开"),
            Viewpoint::new("柜台特写", "柜台台面局部，铜壶和茶杯"),
        ];
        let a = grid_prompt("老茶馆", topic, &views, want);

        // E：Anti 的英文版式措辞 + 逐格点名。**两者的长处合起来。**
        let cells: Vec<String> = views
            .iter()
            .enumerate()
            .map(|(i, v)| format!("panel {}: {} — {}", i + 1, v.name, v.description))
            .collect();
        let e = format!(
            "a 3x3 grid contact sheet of environment concept art, 9 cinematic thumbnails of \
             the SAME location, clean white borders separating panels, master visual study, \
             no people, no text. 场景：{topic}。每一格拍的是同一个空间的不同机位：\n{}",
            cells.join("\n")
        );

        // B：Anti 提的英文 contact sheet 写法 —— 这些是真实的摄影/美术术语，
        // 在训练语料里出现过，比中文结构化描述更可能命中版式
        let b = "a 3x3 grid contact sheet of environment concept art, 9 cinematic scene                  thumbnails of a 1930s Chinese teahouse with wooden beams, grey brick floor                  and a row of latticed windows in warm afternoon light, multiple camera angles,                  wide shots and close-ups, clean white borders separating panels,                  master visual study, no people, no text";

        // C：英文版式 + 中文内容 —— 看版式那句是不是关键
        let c = format!(
            "a 3x3 grid contact sheet, 9 thumbnails in a 3 by 3 layout,              clean white borders separating panels. 每一格是同一个场景的不同机位：{topic}。             画面里不要出现人物和文字。"
        );

        // **没有 FLUX2。** 它接过一天就被 Krea 2 顶掉了，权重和引擎变体都删了。
        // 换 Krea 2 进来反而更对题：它的提示词服从度是四个引擎里最强的
        // （实测把「三个青苹果」数对了，Z-Image 出成 4 个），
        // 而「画成精确的 3×3」本质就是服从一条指令。
        for (name, prompt, engine) in [
            ("A_中文结构化_zimage", a.as_str(), "zimage"),
            ("B_英文contactsheet_zimage", b, "zimage"),
            ("C_英文版式中文内容_zimage", c.as_str(), "zimage"),
            ("D_英文contactsheet_krea2", b, "krea2"),
            ("E_英文版式加逐格点名_zimage", e.as_str(), "zimage"),
        ] {
            let out = std::path::PathBuf::from(format!("/tmp/anima-grid-ab/{name}.png"));
            std::fs::create_dir_all(out.parent().unwrap()).unwrap();
            let (w, h) = (1536u32, 1024u32);
            let graph = match engine {
                // Krea 2 的默认步数是 8（Turbo），和 Z-Image 一样 —— 别按 SDXL 的 20 填
                "krea2" => crate::agent_runtime::image_engine::build_krea2_graph(
                    prompt, w, h, 20_260_914, 8,
                ),
                _ => crate::agent_runtime::image_engine::build_zimage_graph(
                    prompt, w, h, 20_260_914, 8,
                ),
            };
            match submit_one(&graph, &out).await {
                Ok(()) => {
                    let got = image::open(&out).ok().and_then(|i| detect_grid(&i));
                    let hit = got == Some(want);
                    println!(
                        "  {name:<28} → {got:?}  {}",
                        if hit { "✅ 对上了" } else { "❌ 没对上" }
                    );
                }
                Err(e) => println!("  {name:<28} → 失败：{e}"),
            }
        }
        println!("\n判据只有一个：认出来的行列数 == 要求的 3×3。图在 /tmp/anima-grid-ab/");
    }

    /// 提交一张图并落盘。对照实验专用。
    async fn submit_one(
        graph: &serde_json::Value,
        out: &std::path::Path,
    ) -> Result<(), String> {
        const URL: &str = "http://127.0.0.1:8188";
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let res: serde_json::Value = client
            .post(format!("{URL}/prompt"))
            .json(&serde_json::json!({ "prompt": graph }))
            .send().await.map_err(|e| e.to_string())?
            .json().await.map_err(|e| e.to_string())?;
        let id = res["prompt_id"].as_str().ok_or("没拿到 prompt_id")?.to_string();
        for _ in 0..600 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let h: serde_json::Value = client
                .get(format!("{URL}/history/{id}"))
                .send().await.map_err(|e| e.to_string())?
                .json().await.unwrap_or(serde_json::Value::Null);
            if let Some(entry) = h.get(&id) {
                let img = entry["outputs"].as_object()
                    .and_then(|o| o.values().find_map(|v| v["images"].as_array()?.first().cloned()))
                    .ok_or("历史里没有图")?;
                let bytes = client
                    .get(format!("{URL}/view?filename={}&subfolder={}&type={}",
                        img["filename"].as_str().unwrap_or(""),
                        img["subfolder"].as_str().unwrap_or(""),
                        img["type"].as_str().unwrap_or("output")))
                    .send().await.map_err(|e| e.to_string())?
                    .bytes().await.map_err(|e| e.to_string())?;
                std::fs::write(out, &bytes).map_err(|e| e.to_string())?;
                return Ok(());
            }
        }
        Err("等超时了".into())
    }

    /// 拿真实出的那张重切，**验四边没有白条**。
    #[test]
    #[ignore = "要先跑过 scene_sheet_live"]
    fn 真实切格不带分隔线白边() {
        let sheet = std::path::Path::new("/tmp/anima-scene-sheet/sheet.png");
        if !sheet.is_file() {
            println!("跳过（还没出过图）");
            return;
        }
        let views = vec![
            Viewpoint::new("全景", ""), Viewpoint::new("正打", ""),
            Viewpoint::new("反打", ""), Viewpoint::new("窗边", ""),
        ];
        let cells = split_sheet(sheet, Grid { rows: 2, cols: 2 }, &views).unwrap();
        for (i, p) in cells.iter().enumerate() {
            let c = image::open(p).unwrap().to_rgb8();
            let (w, h) = c.dimensions();
            let white = |px: &image::Rgb<u8>| px.0.iter().all(|&v| v >= 235);
            // 抽几条线看，不只看一个点 —— 白边是整条的
            let edge_white = (0..h).step_by(17).filter(|&y| white(c.get_pixel(0, y))).count();
            println!("  第 {} 格 {}×{}，左边白点 {}", i + 1, w, h, edge_white);
            assert!(edge_white * 4 < (h / 17) as usize, "第 {} 格左边还是白条", i + 1);
        }
    }

    /// 拿真实切出来的一格跑精修，**看构图保不保得住**。
    ///
    /// 精修唯一的失败样子是「重画了」—— 出来一张单看很好的图，
    /// 但物体位置变了、和别的格子对不上。而那**不报错**。
    #[tokio::test]
    #[ignore = "要先跑过 scene_sheet_live"]
    async fn scene_sheet_live_逐格精修() {
        let cell = std::path::Path::new("/tmp/anima-scene-sheet/sheet_2_正打.png");
        if !cell.is_file() {
            println!("跳过（还没切过图）");
            return;
        }
        let img = image::open(cell).unwrap();
        let plan = refine_plan(img.width(), img.height(), 1024, 0.35);
        println!(
            "原图 {}×{} → 精修到 {}×{}，denoise {}",
            img.width(), img.height(), plan.width, plan.height, plan.denoise
        );
        let prompt = refine_prompt(
            "民国风格的老茶馆，木梁、青砖地，午后斜光",
            "从店堂中央看向后墙的柜台",
        );
        println!("提示词：{prompt}\n");

        // 源图搬进 ComfyUI 的 input
        let name = format!("anima_refine_{}.png", uuid::Uuid::new_v4());
        let mut copied = false;
        for dir in crate::agent_runtime::video_tool::find_comfyui_input_dirs() {
            copied |= std::fs::copy(cell, dir.join(&name)).is_ok();
        }
        assert!(copied, "源图搬不进 ComfyUI input");

        let mut graph = crate::agent_runtime::image_engine::build_zimage_graph(
            &prompt, plan.width, plan.height, 20_260_913, 8,
        );
        assert!(
            crate::agent_runtime::image_engine::attach_i2i(&mut graph, &name, plan.denoise),
            "i2i 没挂上 —— 挂不上的话它会当成文生图，构图全丢"
        );

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        const URL: &str = "http://127.0.0.1:8188";
        let res: serde_json::Value = client
            .post(format!("{URL}/prompt"))
            .json(&serde_json::json!({ "prompt": graph }))
            .send().await.expect("提交失败")
            .json().await.expect("返回不是 JSON");
        let id = res["prompt_id"].as_str().expect("没拿到 prompt_id").to_string();
        let mut hist = serde_json::Value::Null;
        for _ in 0..600 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let h: serde_json::Value = client
                .get(format!("{URL}/history/{id}"))
                .send().await.expect("查历史失败")
                .json().await.unwrap_or(serde_json::Value::Null);
            if h.get(&id).is_some() { hist = h; break; }
        }
        let entry = hist.get(&id).expect("等超时了");
        let out_img = entry["outputs"].as_object()
            .and_then(|o| o.values().find_map(|v| v["images"].as_array()?.first().cloned()))
            .expect("历史里没有图");
        let bytes = client
            .get(format!("{URL}/view?filename={}&subfolder={}&type={}",
                out_img["filename"].as_str().unwrap_or(""),
                out_img["subfolder"].as_str().unwrap_or(""),
                out_img["type"].as_str().unwrap_or("output")))
            .send().await.expect("取图失败")
            .bytes().await.expect("取图失败");
        let out = std::path::PathBuf::from("/tmp/anima-scene-sheet/refined.png");
        std::fs::write(&out, &bytes).unwrap();
        println!("精修结果：{}", out.display());
        println!("\n**必须真的看**：柜台、门、窗的位置还在不在原处。");
        println!("单看好不好不是判据 —— 重画掉的那张也会很好看。");
    }

    #[tokio::test]
    #[ignore = "要本机 ComfyUI"]
    async fn scene_sheet_live_出一张宫格并切开() {
        // **视角描述写成「两两关系 + 视线终点」**，不是「面向柜台的正面机位」
        // 这种笼统说法 —— 上一轮实测就是笼统说法导致全景和反打出成了同一机位。
        let views = vec![
            Viewpoint::new("全景", "站在临街木门内侧，朝店堂深处看，尽头是靠后墙的柜台"),
            Viewpoint::new("正打", "从店堂中央看向后墙的柜台，柜台占画面中心，两侧是木格窗"),
            Viewpoint::new("反打", "站在柜台后面朝门口看，画面尽头是刚才那扇临街木门，柜台在身后看不见"),
            Viewpoint::new("窗边", "贴着临街那排木格窗往里看，窗在画面左侧，桌椅沿窗一字排开"),
        ];
        let grid = grid_for(views.len());
        let (w, h) = (1536u32, 1024u32);
        let prompt = grid_prompt(
            "老茶馆",
            "民国风格的老茶馆，木梁、青砖地、临街一排木格窗，午后斜光",
            &views,
            grid,
        );
        println!("\n=== 宫格提示词 ===\n{prompt}\n");
        println!("=== 像素预算 ===\n{}\n", PixelBudget::of(w, h, grid).note());

        let out = std::path::PathBuf::from("/tmp/anima-scene-sheet/sheet2.png");
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();

        let graph =
            crate::agent_runtime::image_engine::build_zimage_graph(&prompt, w, h, 20_260_911, 8);

        println!("提交中（Z-Image 约 45 秒）…");
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        const URL: &str = "http://127.0.0.1:8188";
        let res: serde_json::Value = client
            .post(format!("{URL}/prompt"))
            .json(&serde_json::json!({ "prompt": graph }))
            .send()
            .await
            .expect("提交失败")
            .json()
            .await
            .expect("提交返回不是 JSON");
        let id = res["prompt_id"].as_str().expect("没拿到 prompt_id").to_string();

        // 轮询到出结果。**判据是「ComfyUI 还认不认这个任务」**，不是固定秒数 ——
        // 固定预算必然误判，实测耗时是稳态值，不含模型加载和别人塞的任务。
        let mut hist = serde_json::Value::Null;
        for _ in 0..600 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let h: serde_json::Value = client
                .get(format!("{URL}/history/{id}"))
                .send()
                .await
                .expect("查历史失败")
                .json()
                .await
                .unwrap_or(serde_json::Value::Null);
            if h.get(&id).is_some() {
                hist = h;
                break;
            }
        }
        let entry = hist.get(&id).expect("等超时了，任务没出结果");
        let img = entry["outputs"]
            .as_object()
            .and_then(|o| o.values().find_map(|v| v["images"].as_array()?.first().cloned()))
            .expect("历史里没有图");
        let bytes = client
            .get(format!(
                "{URL}/view?filename={}&subfolder={}&type={}",
                img["filename"].as_str().unwrap_or(""),
                img["subfolder"].as_str().unwrap_or(""),
                img["type"].as_str().unwrap_or("output")
            ))
            .send()
            .await
            .expect("取图失败")
            .bytes()
            .await
            .expect("取图失败");
        std::fs::write(&out, &bytes).unwrap();
        println!("联合图：{} ({:.1} MB)", out.display(), bytes.len() as f64 / 1e6);

        let cells = split_sheet(&out, grid, &views).expect("切分失败");
        for c in &cells {
            println!("  {}", c.display());
        }
        assert_eq!(cells.len(), 4);
        println!("\n**必须真的看**：四格是不是同一个空间、光线方向一致不一致。");
        println!("切出来的图在 /tmp/anima-scene-sheet/");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vps(n: usize) -> Vec<Viewpoint> {
        (0..n).map(|i| Viewpoint::new(format!("视角{i}"), "")).collect()
    }

    /// **空场禁令要把人会以什么形式溜进来都点名。**
    ///
    /// 抄的是 liblib 那份场景规范。原来只有一句笼统的「不要出现任何人物」，
    /// 而「剪影」「倒影」「影子」这三种最容易被当成氛围留下来 ——
    /// 它们一样会把这张空场板绑死在某一场戏上。
    #[test]
    fn 空场禁令点名到具体形态() {
        let p = grid_prompt("", "大殿", &default_viewpoints(), grid_for(4));
        for w in ["剪影", "倒影", "影子", "屏幕里的人", "镜中的人"] {
            assert!(p.contains(w), "空场禁令漏了「{w}」");
        }
        // 固定陈设要留下 —— 那是地理的一部分，不是人
        assert!(p.contains("不可移动物要保留"), "{p}");
    }

    /// **畸变禁令不能少，我们比 liblib 更需要它。**
    ///
    /// 方位词条里写死了 24mm 广角（`sceneViews.ts` 的 `lens: "f24"`）——
    /// 不说「不要广角畸变」，等于一边要广角一边不管画面拉不拉变形。
    #[test]
    fn 有畸变禁令和深景深要求() {
        let p = grid_prompt("", "大殿", &default_viewpoints(), grid_for(4));
        for w in ["广角畸变", "鱼眼", "极端俯视", "景深要深"] {
            assert!(p.contains(w), "漏了「{w}」");
        }
    }

    /// **一致性清单要包含「建筑结构、空间比例、主体位置」。**
    ///
    /// 原来只覆盖光源 / 色温 / 材质 —— 而四个视角变成四个不同大小的房间
    /// 是实拍撞到过的（现代办公室那一组）。
    #[test]
    fn 一致性清单管到空间本身() {
        let p = grid_prompt("", "大殿", &default_viewpoints(), grid_for(4));
        assert!(p.contains("建筑结构、空间比例、主体位置"), "{p}");
        assert!(p.contains("不得让四个视角变成四个不同的场景"), "{p}");
    }

    #[test]
    fn 宫格按视角数量选形状() {
        assert_eq!(grid_for(1), Grid { rows: 1, cols: 1 });
        assert_eq!(grid_for(2), Grid { rows: 1, cols: 2 });
        assert_eq!(grid_for(4), Grid { rows: 2, cols: 2 });
        assert_eq!(grid_for(6), Grid { rows: 2, cols: 3 });
        assert_eq!(grid_for(9), Grid { rows: 3, cols: 3 });
    }

    /// **不用 1×N 长条。** 那是设定卡的形状，每格会被压得很扁 ——
    /// 整排图的脸只有 40~46 px 就是这么来的。
    #[test]
    fn 不会退化成一行长条() {
        for n in 3..=9 {
            let g = grid_for(n);
            assert!(g.rows > 1, "{n} 个视角排成了一行：{g:?}");
        }
    }

    #[test]
    fn 超过九个视角分页() {
        assert_eq!(paginate(&vps(0)).len(), 0);
        assert_eq!(paginate(&vps(9)).len(), 1);
        assert_eq!(paginate(&vps(10)).len(), 2);
        assert_eq!(paginate(&vps(19)).len(), 3);
        assert_eq!(paginate(&vps(10))[1].len(), 1);
    }

    /// **像素预算是这条路最容易翻车的地方。**
    ///
    /// 本机 Z-Image 出 1536×1024，3×3 每格 512×341 —— 短边 341，
    /// 低于门槛，只能当布局稿。设定卡那次就是没人算这笔账。
    #[test]
    fn 本机典型尺寸下的三乘三是布局稿不是成品() {
        let b = PixelBudget::of(1536, 1024, Grid { rows: 3, cols: 3 });
        assert_eq!((b.cell_width, b.cell_height), (512, 341));
        assert_eq!(b.short_side(), 341);
        assert!(!b.usable_as_final(), "341 短边不该被当成品");
        assert!(b.note().contains("布局稿"), "{}", b.note());
        assert!(b.note().contains("精修"), "得说清楚下一步：{}", b.note());
    }

    #[test]
    fn 格子够大时才算成品() {
        // 2×2 在同一张图上每格 768×512 —— 够用
        let b = PixelBudget::of(1536, 1024, Grid { rows: 2, cols: 2 });
        assert_eq!((b.cell_width, b.cell_height), (768, 512));
        assert!(b.usable_as_final());
        assert!(b.note().contains("可直接使用"));
    }

    #[test]
    fn 每格逐个点名而不是笼统说不同视角() {
        let vp = vec![
            Viewpoint::new("全景", "整个房间"),
            Viewpoint::new("反打", "看见门"),
        ];
        let p = grid_prompt("老茶馆", "午后光线", &vp, Grid { rows: 1, cols: 2 });
        // 版式写在最前，用英文摄影术语（实测比中文「N 行 M 列」更容易命中版式）
        assert!(p.starts_with("a 2x1 grid contact sheet"), "版式要写在最前：{p}");
        assert!(p.contains("clean white borders separating panels"), "{p}");
        assert!(p.contains("第 1 格：全景 —— 整个房间"));
        assert!(p.contains("第 2 格：反打 —— 看见门"));
        assert!(p.contains("午后光线"));
        assert!(p.contains("同一个空间"), "一致性约束丢了：{p}");
    }

    /// 空格子不说清楚，模型会自己找东西填满 —— 那几格是废的，而且不报错。
    #[test]
    fn 视角不满时明确要求留空() {
        let p = grid_prompt("客厅", "", &vps(7), Grid { rows: 3, cols: 3 });
        assert!(p.contains("剩下 2 个格子留空白"), "{p}");
        // 刚好填满时不要多这一句
        let p2 = grid_prompt("客厅", "", &vps(9), Grid { rows: 3, cols: 3 });
        assert!(!p2.contains("留空白"), "{p2}");
    }

    /// **场景板默认空场。**
    ///
    /// 它的价值是「后续镜头共享同一可导航空间」—— 带上角色就绑死在某一场戏上，
    /// 不能复用了。
    #[test]
    fn 场景板默认不出现人物() {
        let p = grid_prompt("老茶馆", "午后", &vps(4), Grid { rows: 2, cols: 2 });
        assert!(p.contains("不要出现任何人物"), "{p}");
        assert!(p.contains("固定家具"), "家具要留着，那是地理的一部分：{p}");
    }

    /// **跨视角的光线一致性要逐项点名。**
    ///
    /// 地理错位一眼能看出来，而色温和主光位不一致常常各自都「很好看」，
    /// 只在两个视角被剪在一起时才暴露 —— 逐张单独看抓不到。
    /// **场景名不能被画进画面。**
    ///
    /// 实测撞过：只写「不要写文字标注」时，模型把它理解成「不要标注格子编号」，
    /// 转头把场景名当成招牌画到了墙上（四格里有两格墙上写着「老茶馆」）。
    /// 场景板是地理参考，画上招牌就把通用空间钉死成了某一家店。
    #[test]
    /// **场景名一个字都不进提示词。**
    ///
    /// 2026-09-11 实测：同时写了「这个名字只用于理解，不要把它写进画面」、
    /// 「画面里不要出现任何文字：没有招牌…」，契约的负向清单里还有
    /// 「任何招牌文字」—— 三道禁令，四格里仍有两格把「安和面馆」
    /// 当招牌画在了门楣上。
    ///
    /// 上一次「修」这个问题是把禁令拆成两句说清楚，那没用。根因是名字
    /// 本身进了提示词，而 Z-Image 认得中文也画得出中文，负向词对它又
    /// 结构性无效（cfg=1）。名字对画面没有贡献，不发就不会被画出来。
    fn 场景名一个字都不发给模型() {
        let p = grid_prompt("老茶馆", "木梁青砖", &vps(4), Grid { rows: 2, cols: 2 });
        assert!(!p.contains("老茶馆"), "场景名漏进提示词了：{p}");
        // 文字禁令仍然要有 —— 菜单、匾额那些不是我们递给它的，是它自己加的
        assert!(p.contains("不要出现任何文字"), "{p}");
        assert!(p.contains("招牌"), "要把招牌点出来 —— 那是实测里真画上去的东西：{p}");
    }

    /// 空场政策不重复。场景描述自己说了就不再说一遍 ——
    /// 空间契约的 `to_prompt_block` 带着它自己的空场政策，实测撞过同一句
    /// 话在提示词里连着出现两遍。
    #[test]
    fn 空场政策不重复() {
        let desc = "场景「面馆」：狭长\n画面里**不要出现任何人物**（固定家具、灯具、设备等不可移动物要保留）。";
        let p = grid_prompt("面馆", desc, &vps(4), Grid { rows: 2, cols: 2 });
        assert_eq!(p.matches("不要出现任何人物").count(), 1, "{p}");
        // 描述里没说时照样要发
        let bare = grid_prompt("面馆", "狭长", &vps(4), Grid { rows: 2, cols: 2 });
        assert_eq!(bare.matches("不要出现任何人物").count(), 1, "{bare}");
    }

    /// 反打要真的转到相对一侧。实测里它出成了同一机位挪一点。
    #[test]
    fn 要求反打真的转到相对一侧() {
        let p = grid_prompt("客厅", "", &vps(4), Grid { rows: 2, cols: 2 });
        assert!(p.contains("反打格必须真的转到相对的一侧"), "{p}");
        assert!(p.contains("空间关系"), "{p}");
    }

    #[test]
    fn 一致性清单逐项点名而不是笼统说同一个空间() {
        let p = grid_prompt("客厅", "", &vps(4), Grid { rows: 2, cols: 2 });
        for item in ["主光来源", "色温关系", "光比", "暗区", "开关状态", "材质"] {
            assert!(p.contains(item), "一致性清单缺「{item}」：{p}");
        }
        // 正常的方向性差异不该被拉平
        assert!(p.contains("逆光"), "该说明哪些差异是正常的：{p}");
    }

    /// 视角表三种写法都要认，而且**空的要回落到默认**而不是切出零格。
    #[test]
    fn 视角表三种写法都认() {
        use serde_json::json;
        // 字符串数组
        let v = json!(["全景", "反打"]);
        let got = viewpoints_from_args(Some(&v));
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].name, "全景");
        assert_eq!(got[0].description, "");

        // 对象数组
        let v = json!([{"name": "全景", "description": "整个房间"}]);
        let got = viewpoints_from_args(Some(&v));
        assert_eq!(got[0].description, "整个房间");

        // 没给 / 空的 / 全是垃圾 → 默认
        assert_eq!(viewpoints_from_args(None), default_viewpoints());
        assert_eq!(viewpoints_from_args(Some(&json!([]))), default_viewpoints());
        assert_eq!(viewpoints_from_args(Some(&json!(["  ", {}]))), default_viewpoints());
    }

    /// **真的切一张图，不是只算几何。**
    ///
    /// 用纯色块拼一张 2×2，切完逐格验颜色 —— 算对了坐标不代表切对了格子，
    /// 行列搞反的话几何测试照样通过。
    /// **按图里实际画成什么样切，不按我们要求的样子切。**
    ///
    /// 实测撞过：要 2×2 出了 3×2，而按 2×2 切照样「成功」——
    /// 每块横跨两个格子，文件都在、数量也对，就是内容全错。
    /// **重绘强度要夹紧，超过就把构图重画掉了。**
    ///
    /// 项目里 i2i 的默认值是 0.65 —— 那是**修图**（用户想让它改）的场景。
    /// 精修不能用那个数：构图正是第一段唯一的产出。
    #[test]
    fn 重绘强度夹在精修的范围内() {
        assert_eq!(refine_plan(512, 512, 1024, 0.65).denoise, MAX_REFINE_DENOISE);
        assert_eq!(refine_plan(512, 512, 1024, 1.0).denoise, MAX_REFINE_DENOISE);
        assert_eq!(refine_plan(512, 512, 1024, 0.35).denoise, 0.35);
        // 太低也没意义，但不该变成 0
        assert!(refine_plan(512, 512, 1024, 0.0).denoise > 0.0);
    }

    /// **放大最多 2 倍。**
    ///
    /// i2i 补的是细节不是分辨率 —— 超过 2 倍模型没有足够的像素依据，
    /// 出来的是「涂抹出来的细节」，还容易改掉物体形状。
    #[test]
    fn 放大倍数夹在两倍以内() {
        // 512 → 想要 2048（4 倍），夹到 1024
        let p = refine_plan(512, 512, 2048, 0.35);
        assert_eq!(p.width, 1024);
        assert_eq!(p.height, 1024);
        // 目标比原图还小时不缩小 —— 精修不该降分辨率
        let p = refine_plan(768, 512, 256, 0.35);
        assert_eq!((p.width, p.height), (768, 512));
    }

    /// **保持宽高比。** 改了比例构图就变了，而构图是第一段唯一的产出。
    #[test]
    fn 精修保持宽高比并按三十二对齐() {
        // 本机 3×3 的典型格子：512×341
        let p = refine_plan(512, 341, 1024, 0.35);
        assert_eq!(p.width, 1024);
        // 341 × 2 = 682 → 32 对齐到 672
        assert_eq!(p.height, 672);
        assert_eq!(p.width % 32, 0, "宽没有 32 对齐");
        assert_eq!(p.height % 32, 0, "高没有 32 对齐");
        // 比例偏差要小 —— 32 对齐必然有一点误差，但不能把 3:2 变成 16:9
        let before = 512.0 / 341.0;
        let after = p.width as f64 / p.height as f64;
        assert!((before - after).abs() < 0.05, "比例跑了：{before} → {after}");
    }

    /// **精修不加新东西。**
    /// 加新元素会让这一格和别的格子分家 —— 而九格一致正是第一段的产出，
    /// 第二段的职责是保住它。
    #[test]
    fn 精修提示词只重复不新增() {
        let p = refine_prompt("水彩插画，冷调", "从入口看向柜台");
        assert!(p.contains("从入口看向柜台"), "{p}");
        assert!(p.contains("水彩插画，冷调"), "{p}");
        assert!(p.contains("保持原构图"), "{p}");
        assert!(p.contains("只补足材质"), "{p}");
        // 什么都没有时也要给一句能用的
        assert!(refine_prompt("", "").contains("保持原构图"));
    }

    #[test]
    fn 从图里实测格子布局() {
        use image::{Rgb, RgbImage};
        // 造一张 3×2：内容深色，分隔线纯白
        let (w, h) = (300u32, 200u32);
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                // 每 100 宽、100 高一格，格间留 6px 白线
                let cx = x % 100;
                let cy = y % 100;
                let sep = !(3..=96).contains(&cx) || !(3..=96).contains(&cy);
                img.put_pixel(x, y, if sep { Rgb([255, 255, 255]) } else { Rgb([40, 40, 40]) });
            }
        }
        let d = image::DynamicImage::ImageRgb8(img);
        assert_eq!(detect_grid(&d), Some(Grid { rows: 2, cols: 3 }));
    }

    /// 模型画成一张普通的图（没有分隔线）时**不硬切**。
    #[test]
    fn 没有分隔线时拒绝切分而不是切出碎片() {
        use image::{Rgb, RgbImage};
        let dir = std::env::temp_dir().join(format!("anima-sheet-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let sheet = dir.join("plain.png");
        let mut img = RgbImage::new(200, 200);
        for p in img.pixels_mut() {
            *p = Rgb([80, 90, 100]);
        }
        img.save(&sheet).unwrap();

        let got = split_sheet(&sheet, Grid { rows: 2, cols: 2 }, &vps(4));
        let err = got.expect_err("没有分隔线就不该切");
        assert!(err.contains("找不到宫格分隔线"), "{err}");
        assert!(err.contains("横跨格子"), "要说清楚为什么不切：{err}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 真的把图切成对应的格子() {
        use image::{Rgb, RgbImage};
        let dir = std::env::temp_dir().join(format!("anima-sheet-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let sheet = dir.join("scene.png");

        // 左上红、右上绿、左下蓝、右下黄，**格间留白线** ——
        // 真实宫格就是这样，而切分器现在会从分隔线实测布局
        let mut img = RgbImage::new(100, 100);
        for y in 0..100u32 {
            for x in 0..100u32 {
                let sep = (48..52).contains(&x) || (48..52).contains(&y);
                let c = if sep {
                    Rgb([255, 255, 255])
                } else {
                    match (x < 50, y < 50) {
                        (true, true) => Rgb([255, 0, 0]),
                        (false, true) => Rgb([0, 255, 0]),
                        (true, false) => Rgb([0, 0, 255]),
                        (false, false) => Rgb([255, 220, 0]),
                    }
                };
                img.put_pixel(x, y, c);
            }
        }
        img.save(&sheet).unwrap();

        let vp = vec![
            Viewpoint::new("左上", ""), Viewpoint::new("右上", ""),
            Viewpoint::new("左下", ""), Viewpoint::new("右下", ""),
        ];
        let cells = split_sheet(&sheet, Grid { rows: 2, cols: 2 }, &vp).unwrap();
        assert_eq!(cells.len(), 4);

        // **文件名带序号和视角名** —— 下游要按名索引
        assert!(cells[0].to_string_lossy().contains("scene_1_左上"), "{:?}", cells[0]);
        assert!(cells[3].to_string_lossy().contains("scene_4_右下"), "{:?}", cells[3]);

        // 逐格验颜色：行列搞反的话这里会红
        let expect = [[255u8, 0, 0], [0, 255, 0], [0, 0, 255], [255, 220, 0]];
        for (i, p) in cells.iter().enumerate() {
            let c = image::open(p).unwrap().to_rgb8();
            // **48 不是 50** —— 分隔线（48..52）被排除在格子之外了。
            // 均分的话每格 50×50，会把 2px 白边切进来。
            assert_eq!(c.dimensions(), (48, 48), "第 {} 格尺寸不对", i + 1);
            let px = c.get_pixel(24, 24).0;
            assert_eq!(px, expect[i], "第 {} 格切错了位置", i + 1);

            // **四条边都不能是白的。**
            //
            // 白边会一路进精修、进参考图、进首帧，最后变成画面里的一道白框，
            // 而图本身「看起来是好的」。实测撞到过：精修出来的图四周有白框。
            let (w, h) = c.dimensions();
            let white = |px: &image::Rgb<u8>| px.0.iter().all(|&v| v >= 235);
            assert!(!white(c.get_pixel(0, h / 2)), "第 {} 格左边是白的", i + 1);
            assert!(!white(c.get_pixel(w - 1, h / 2)), "第 {} 格右边是白的", i + 1);
            assert!(!white(c.get_pixel(w / 2, 0)), "第 {} 格上边是白的", i + 1);
            assert!(!white(c.get_pixel(w / 2, h - 1)), "第 {} 格下边是白的", i + 1);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// 引擎没按要求出尺寸时，装不下的格子跳过，**不 panic 也不整次失败**。
    #[test]
    fn 单格不走宫格检测() {
        use image::RgbImage;
        let dir = std::env::temp_dir().join(format!("anima-sheet-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let sheet = dir.join("small.png");
        RgbImage::new(60, 20).save(&sheet).unwrap();
        // **1×1 不走检测** —— 单视角本来就是一张普通的图，没有分隔线
        let got = split_sheet(&sheet, Grid { rows: 1, cols: 1 }, &vps(1));
        assert!(got.is_ok(), "单格不该被守卫拦下：{got:?}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 文件名里的非法字符被剥掉() {
        use image::RgbImage;
        let dir = std::env::temp_dir().join(format!("anima-sheet-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let sheet = dir.join("s.png");
        RgbImage::new(20, 20).save(&sheet).unwrap();
        let vp = vec![Viewpoint::new("门口/走廊", "")];
        let cells = split_sheet(&sheet, Grid { rows: 1, cols: 1 }, &vp).unwrap();
        assert!(!cells[0].to_string_lossy().contains("门口/走廊"));
        assert!(cells[0].to_string_lossy().contains("门口走廊"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 切分几何按行列铺开() {
        let cells = cells_of(1536, 1024, Grid { rows: 2, cols: 2 }, &vps(4));
        assert_eq!(cells.len(), 4);
        assert_eq!((cells[0].x, cells[0].y), (0, 0));
        assert_eq!((cells[1].x, cells[1].y), (768, 0));
        assert_eq!((cells[2].x, cells[2].y), (0, 512));
        assert_eq!((cells[3].x, cells[3].y), (768, 512));
        assert!(cells.iter().all(|c| c.width == 768 && c.height == 512));
    }

    /// 格子不整除时**不按比例分** —— 那样每格差一两像素，拼回去对不上。
    #[test]
    fn 尺寸不整除时每格等宽() {
        let cells = cells_of(1000, 1000, Grid { rows: 3, cols: 3 }, &vps(9));
        assert!(cells.iter().all(|c| c.width == 333 && c.height == 333));
        assert_eq!(cells[8].x, 666);
    }

    #[test]
    fn 视角比格子多时只切得下的那些() {
        let cells = cells_of(600, 600, Grid { rows: 2, cols: 2 }, &vps(6));
        assert_eq!(cells.len(), 4);
    }

    /// **格子比视角多时要切满，不按视角数截断。**
    ///
    /// 实测撞过：要 3×3 九格，模型画了 4×3 十二格。按视角数只切出前 9 张的话，
    /// 调用方一看「9 张对 9 个镜头」就放行了 —— 而模型把内容铺在 12 格里，
    /// 第 6 镜的画面落在第 8 格。**每张单看都是合理的画面**，没人看得出来。
    /// 切满之后数量自然对不上，调用方的计数守卫才拦得住。
    #[test]
    fn 格子比视角多时切满实际格子数() {
        let cells = cells_of(300, 400, Grid { rows: 4, cols: 3 }, &vps(9));
        assert_eq!(cells.len(), 12, "该切满 12 格，让调用方发现对不上");
        // 前 9 格用视角名，多出来的有能认的名字
        assert_eq!(cells[0].name, "视角0");
        assert_eq!(cells[9].name, "格10");
        assert_eq!(cells[11].name, "格12");
    }
}
