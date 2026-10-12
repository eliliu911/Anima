//! 场景空间契约 —— 用**文字地图**锁住地理，不靠俯视图。
//!
//! # 它解决的是「九个格子真的是同一个环境吗」
//!
//! 场景宫格（[`crate::agent_runtime::scene_sheet`]）保证了九个格子出自
//! **同一次采样**，所以光线、材质、家具款式必然同源。但它保证不了
//! **地理**：柜台和门的相对位置对不对、反打有没有真的转到对面。
//!
//! 实测撞过：四格里「全景」和「反打」出成了几乎同一个机位，光向和材质
//! 全对上了，**柜台在第 2 格有、第 3 格同角度却没有**。
//!
//! `scene_sheet::grid_prompt` 的注释把这件事挂在了调用方头上——
//! 「视角描述要写成『两两关系 + 视线终点』，那是调用方的责任」。
//! 这个模块就是那个调用方。
//!
//! # 为什么是文字地图，不是俯视图
//!
//! 生成一张俯视平面图去核对那九张图，是**用不可靠的东西验不可靠的东西**——
//! 俯视图自己也是采样出来的，它不比九个格子更权威。而且从平面图推透视
//! 不是当前工具链干得了的事：ControlNet 的 depth / pose / canny 全是
//! **同视角**约束。它看起来像解决方案，实际给不了那个保证，
//! 反而容易让人以为验过了。
//!
//! 能力其实分三层，递减：
//!
//! | 层次 | 手段 | 我们有没有 |
//! |---|---|---|
//! | 保证 | 真几何（3DGS / 全景里摆机位），渲染是确定性的 | ❌ |
//! | 提高概率 | 同一次采样 + **这个模块** | ✅ |
//! | 事后检查 | 色温比 | 有限（见 B 项） |
//!
//! 这个模块待在中间那层。它不保证地理正确，它让地理**有依据可写、
//! 有依据可查** —— 视角描述不再每次现编，换个人写也是同一份地图。
//!
//! # 身份 / 状态分家（抄 drama-skills 的 Location / View）
//!
//! [`SceneContract`] 存**空间身份**：边界、分区、入口、不可移动的锚点、
//! 主要材质。停电、白天黑夜、晴雨、临时布置**都不改变身份** ——
//! 那些是 [`SceneView`] 的 state。只因为镜头转向就新建一个场景，
//! 门窗方向和演员动线会互相矛盾。
//!
//! # 抄 DramaClaw `scene_spatial_contract` 的三处设计
//!
//! - **坐标无关**：位置写成「在柜台后方」「东侧」这种拓扑关系，
//!   不写像素坐标。模型守不住坐标，守得住拓扑。
//! - **负向约束是一等公民**：[`Anchor::must_not_become`] / [`SceneContract::no_extra`]。
//!   真实失败样子是「同一扇门被画成左右各一扇」，正向描述压不住它。
//! - **诚实区分看得见的和编的**：[`SceneContract::inferred`] 单独存。
//!   剧本没写的东西可以补，但不能混进锁死项里当既成事实。

use serde::{Deserialize, Serialize};

/// 锚点的尺度锁。
///
/// **存在的理由是负向的**：不写的话，一个占满整面墙的旧木柜台会在某一格
/// 缩成角落里的小桌子，而那一格单看完全正常。DramaClaw 把它叫 `size_lock`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AnchorSize {
    /// 小物件：门把手、开关、台面上的匣子。
    Small,
    #[default]
    Medium,
    Large,
    /// 占满一整面墙 —— 这一档最需要锁，缩小了最不容易被发现。
    WallDominant,
}

impl AnchorSize {
    pub fn label(self) -> &'static str {
        match self {
            AnchorSize::Small => "小件",
            AnchorSize::Medium => "中等",
            AnchorSize::Large => "大件",
            AnchorSize::WallDominant => "占满整面墙",
        }
    }

    /// 排序权重：越大越该进[`SceneContract::lock_line`]。
    fn weight(self) -> u8 {
        match self {
            AnchorSize::WallDominant => 3,
            AnchorSize::Large => 2,
            AnchorSize::Medium => 1,
            AnchorSize::Small => 0,
        }
    }
}

/// 一个**不可移动**的识别锚点。
///
/// 判据是「搬不走」，不是「显眼」。临时的箱子、海报、积水不是锚点 ——
/// 把临时 dressing 写成永久地理，下一场戏就会自相矛盾。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Anchor {
    pub name: String,
    /// 它在哪个分区。**拓扑名，不是坐标。**
    #[serde(default)]
    pub zone: String,
    #[serde(default)]
    pub size: AnchorSize,
    /// 负向锁：它不能被画成什么。
    ///
    /// 「不要变成现代吧台」比「旧木柜台」有用，因为后者对应的仍然是
    /// 一大片可能性，而前者砍掉了最常见的那个错误落点。
    #[serde(default)]
    pub must_not_become: Vec<String>,
}

/// 入口 / 出口。演员和道具从这里进出，所以它同时是**动线**事实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Entrance {
    pub name: String,
    /// 通向哪里。空着说明剧本没交代，应该进 [`SceneContract::inferred`]。
    #[serde(default)]
    pub connects_to: String,
}

/// 空场政策。抄 location-plate 的 `empty_stage`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EmptyStage {
    /// 一个人都不要。场景板的价值是**复用** —— 带上角色就绑死在某一场戏上。
    #[default]
    Required,
    /// 以空场为主，允许不具身份的尺度参照（远处一个背影）。
    Preferred,
    /// 可以有人，但不能编造具体角色事件。
    NotRequired,
}

impl EmptyStage {
    fn line(self) -> &'static str {
        match self {
            EmptyStage::Required => {
                "画面里**不要出现任何人物**（固定家具、灯具、设备等不可移动物要保留）。"
            }
            EmptyStage::Preferred => {
                "以空场为主；可以有不具身份的远处人影作尺度参照，但不能挡住地理。"
            }
            EmptyStage::NotRequired => "可以有人物活动，但不要编造具体的角色和情节。",
        }
    }
}

/// 一个**稳定可复用的观看方向**。
///
/// 值得建 View 的判据：下游要反复引用它。「低机位」「门把手特写」
/// 属于单个镜头的 framing，不建 View —— 建了会让 View 表变成镜头表。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SceneView {
    /// 短名，进宫格格子标题。
    pub name: String,
    /// 站在哪儿。
    #[serde(default)]
    pub from: String,
    /// 朝哪个方向看。
    #[serde(default)]
    pub toward: String,
    /// **视线终点** —— 这一项最能把两个机位区分开。
    ///
    /// 实测：「面向主要活动区的正面机位」这种笼统说法会和别的格子撞成
    /// 同一个角度；「尽头是靠后墙的柜台」不会。
    #[serde(default)]
    pub far: String,
    /// 这个方向上**同时可见**的锚点。反打能不能咬合就靠它对表。
    #[serde(default)]
    pub visible: Vec<String>,
    /// 主光来源和方向。同一 state 下各 View 必须共享同一套光。
    #[serde(default)]
    pub light: String,
}

impl SceneView {
    /// 这个视角的文字地图，直接进宫格格子的描述位。
    pub fn map_line(&self) -> String {
        let mut bits: Vec<String> = Vec::new();
        let from = self.from.trim();
        let toward = self.toward.trim();
        match (from.is_empty(), toward.is_empty()) {
            (false, false) => bits.push(format!("从{from}朝{toward}看")),
            (false, true) => bits.push(format!("从{from}看")),
            (true, false) => bits.push(format!("朝{toward}看")),
            (true, true) => {}
        }
        let far = self.far.trim();
        if !far.is_empty() {
            bits.push(format!("视线尽头是{far}"));
        }
        let vis: Vec<&str> =
            self.visible.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        if !vis.is_empty() {
            bits.push(format!("同时可见：{}", vis.join("、")));
        }
        let light = self.light.trim();
        if !light.is_empty() {
            bits.push(light.to_string());
        }
        bits.join("，")
    }
}

/// 一个场景的空间身份 + 它的若干观看方向。
///
/// 挂在 [`crate::assets::Asset`] 上，和角色的
/// [`crate::cinema_dna::CharacterVisualDna`] 是对称的两件事：
/// 一个锁人长什么样，一个锁地方长什么样。
/// **不派生 `Eq`**：`pano_calibration` 里是 f64。
/// 契约本来也不该用相等去比 —— 判「有没有内容」走 [`SceneContract::is_empty`]。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SceneContract {
    /// 整体形状和尺度：「东西进深的狭长一层店铺」。
    #[serde(default)]
    pub shape: String,
    /// 功能分区。
    #[serde(default)]
    pub zones: Vec<String>,
    #[serde(default)]
    pub entrances: Vec<Entrance>,
    #[serde(default)]
    pub anchors: Vec<Anchor>,
    /// **两两关系**，不是清单。
    ///
    /// location-plate 的原话：用「两两关系 + 视线终点」而非无序 inventory。
    /// 「配电箱在柜台后墙」是关系；「有配电箱、有柜台」是清单 ——
    /// 后者每一格都能满足，却可以把它们摆在完全不同的地方。
    #[serde(default)]
    pub relations: Vec<String>,
    /// 对辨认结构有用的材质和主次色。
    #[serde(default)]
    pub materials: Vec<String>,
    #[serde(default)]
    pub views: Vec<SceneView>,
    /// **编的**。剧本没写、靠合理推断补上的部分。
    ///
    /// 和锁死项分开存，因为两者的权威性不一样：锁死项冲突了是 bug，
    /// 推断项冲突了可能只是当初推错了。混在一起就分不出来了。
    #[serde(default)]
    pub inferred: Vec<String>,
    /// 全局负向：不要凭空多出来的东西。
    #[serde(default)]
    pub no_extra: Vec<String>,
    #[serde(default)]
    pub empty_stage: EmptyStage,
    /// 这个场景的全景**世界校准**：相机离地多高、一身位多大。
    ///
    /// # 为什么它在这儿，而不在画布节点里
    ///
    /// 全景是生成的 —— 它的相机离地多高、房间多大**我们无从知道**，
    /// 只能让人拖到地面网格贴着地板为止。对一次要花点功夫，
    /// 存在节点里的话换个节点、换一天又得重来。
    ///
    /// # 它严格来说是**每张全景图**的，不是每个场景的
    ///
    /// 同一个场景出两张全景，两张的相机高度未必一样。按场景存是个近似 ——
    /// 取舍是「一个场景通常只有一张在用的全景」。真要按图存得给全景图
    /// 建资产变体并挂在变体上，那是另一件事。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pano_calibration: Option<PanoCalibration>,
    /// 这个地点的 **3D 片场**（3DGS）。一个地点生成一次，在这儿的每一镜都用它（导演世界、出首帧）。
    ///
    /// 和 `pano_calibration` 同样的取舍：严格说是「这张全景图的」，按场景存是近似。
    /// **只由生成命令写**（`studio_director::cinema_generate_scene_world`）；界面整份保存场景契约时
    /// 不带这个字段也不会被冲掉（见 `assets::apply_update`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world: Option<SceneWorld>,
}

/// 一个地点的 3D 片场文件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SceneWorld {
    /// `.ply` / `.ksplat` / `.spz` 的本机路径
    pub path: String,
    pub format: String,
    /// 坐标系。SHARP 出的是 `opencv_camera`（原相机坐标，载入时要翻正，见前端 `scene.splatPlacement`）
    #[serde(default)]
    pub convention: String,
    /// 从哪张图生成的（全景就是全景图本身）
    #[serde(default)]
    pub source_image: String,
    #[serde(default)]
    pub created_at: u64,
    /// 片场缩放。SHARP 的单位只是「大约米」：拆机间实测相机离地约 1.3、离台面 0.78 单位，
    /// 按 1.6 米眼高算整个片场小了两成左右，和 1.83 米的人偶比例不对（2026-09-27 用户：「比例没有处理好」）。
    /// 在导演世界里拖到比例对为止。没有 = 1。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
}

impl SceneContract {
    /// 除了空间描述以外，还挂着世界数据（校准 / 3D 片场）。这种契约**不能**因为没有锚点就当空的丢掉。
    pub fn has_world_data(&self) -> bool {
        self.pano_calibration.is_some() || self.world.is_some()
    }
}

/// 全景的世界校准。对应 DramaClaw `DirectorWorldSourceTransform` 里
/// 我们用得上的三维：眼高、地面高度和尺度（相机在球心，xz 偏移没有意义）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PanoCalibration {
    /// 相机离地多高。
    pub eye_height: f64,
    /// 网格平面的世界高度；网格就是导演世界里人物真正踩住的地面。
    #[serde(default)]
    pub ground_height: f64,
    /// 一「身位」有多大。
    pub scale: f64,
}

/// [`SceneContract::lock_line`] 最多带几个锚点。
///
/// location-plate：「优先保持一至三个强锚点，次要装饰可概括，
/// 防止提示词在细节噪声中失去布局」。锁面还要进**每一条**提示词，
/// 长了会把真正的画面内容挤掉。
pub const LOCK_ANCHORS: usize = 3;

impl SceneContract {
    /// 有没有值得发出去的内容。
    ///
    /// 空契约比没有契约更糟 —— 它会让人以为地理锁上了。
    pub fn is_empty(&self) -> bool {
        self.anchors.iter().all(|a| a.name.trim().is_empty())
            && self.relations.iter().all(|r| r.trim().is_empty())
            && self.shape.trim().is_empty()
    }

    /// **地理锁 —— 发了参考图也要发。**
    ///
    /// 和角色那条 `lock_line` 同一个道理：参考图只能约束它自己画到的部分。
    /// 一张场景板画的是某一个方向，换一个方向的镜头，柜台在哪、
    /// 有几扇门，图上根本没有 —— 这时候只有文字能带过去。
    ///
    /// 挑最强的几个锚点（尺度优先），带上它们的位置和负向锁，
    /// 再补一句入口数量。**入口数量单独说**，因为「同一扇门被画成
    /// 左右各一扇」是这一类最常见的失败样子。
    pub fn lock_line(&self) -> String {
        let mut picked: Vec<&Anchor> =
            self.anchors.iter().filter(|a| !a.name.trim().is_empty()).collect();
        // 稳定排序：尺度降序，同尺度保持原顺序（作者写在前面的更重要）。
        picked.sort_by_key(|a| std::cmp::Reverse(a.size.weight()));
        picked.truncate(LOCK_ANCHORS);

        let mut parts: Vec<String> = Vec::new();
        for a in picked {
            let mut one = a.name.trim().to_string();
            let zone = a.zone.trim();
            if !zone.is_empty() {
                one.push_str(&format!("在{zone}"));
            }
            if a.size == AnchorSize::WallDominant || a.size == AnchorSize::Large {
                one.push_str(&format!("（{}，不要缩小）", a.size.label()));
            }
            let nots: Vec<&str> = a
                .must_not_become
                .iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            if !nots.is_empty() {
                one.push_str(&format!("，不要画成{}", nots.join("或")));
            }
            parts.push(one);
        }

        let doors: Vec<&str> = self
            .entrances
            .iter()
            .map(|e| e.name.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if !doors.is_empty() {
            parts.push(format!(
                "入口只有 {} 处（{}），不要再加门或通道",
                doors.len(),
                doors.join("、")
            ));
        }

        if parts.is_empty() {
            return String::new();
        }
        format!("地理锁：{}。", parts.join("；"))
    }

    /// 完整的文字地图。**纯文生图那条路只有它**。
    ///
    /// 和 `lock_line` 的分工与角色那边一致：这一段长，发了参考图就不发
    /// （图已经说清楚了，再写一遍会打架）；锁面短，永远发。
    pub fn to_prompt_block(&self, scene_name: &str) -> String {
        let name = scene_name.trim();
        let mut lines: Vec<String> = Vec::new();

        let shape = self.shape.trim();
        if !name.is_empty() || !shape.is_empty() {
            let head = if shape.is_empty() {
                format!("场景「{name}」")
            } else if name.is_empty() {
                shape.to_string()
            } else {
                format!("场景「{name}」：{shape}")
            };
            lines.push(head);
        }

        let zones: Vec<&str> =
            self.zones.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        if !zones.is_empty() {
            lines.push(format!("分区：{}", zones.join("、")));
        }

        let doors: Vec<String> = self
            .entrances
            .iter()
            .filter(|e| !e.name.trim().is_empty())
            .map(|e| {
                let to = e.connects_to.trim();
                if to.is_empty() {
                    e.name.trim().to_string()
                } else {
                    format!("{}通向{}", e.name.trim(), to)
                }
            })
            .collect();
        if !doors.is_empty() {
            lines.push(format!("入口：{}", doors.join("；")));
        }

        let anchors: Vec<String> = self
            .anchors
            .iter()
            .filter(|a| !a.name.trim().is_empty())
            .map(|a| {
                let mut s = a.name.trim().to_string();
                let zone = a.zone.trim();
                if !zone.is_empty() {
                    s.push_str(&format!("（{zone}）"));
                }
                s
            })
            .collect();
        if !anchors.is_empty() {
            lines.push(format!("不可移动的锚点：{}", anchors.join("、")));
        }

        let rels: Vec<&str> =
            self.relations.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        if !rels.is_empty() {
            lines.push(format!("相对位置：{}", rels.join("；")));
        }

        let mats: Vec<&str> =
            self.materials.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        if !mats.is_empty() {
            lines.push(format!("材质：{}", mats.join("、")));
        }

        let extra: Vec<&str> =
            self.no_extra.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        if !extra.is_empty() {
            lines.push(format!("**不要出现**：{}", extra.join("、")));
        }

        lines.push(self.empty_stage.line().to_string());
        lines.join("\n")
    }

    /// 契约里的视角，转成宫格的格子。
    ///
    /// **这是这个模块存在的直接理由。** 在它之前，宫格的视角描述是
    /// 每次手写的 —— 换个人写就散了，而且没有任何东西保证九个视角
    /// 说的是同一份地理。现在它们全部由同一份契约派生。
    pub fn viewpoints(&self) -> Vec<crate::agent_runtime::scene_sheet::Viewpoint> {
        self.views
            .iter()
            .filter(|v| !v.name.trim().is_empty())
            .map(|v| {
                crate::agent_runtime::scene_sheet::Viewpoint::new(v.name.trim(), v.map_line())
            })
            .collect()
    }

    /// 自检。**返回的是要给人看的话，不是错误码** —— 这些问题里大多数
    /// 不该阻断生成，但必须让人看见。
    ///
    /// 静默失败是这一块的常态：契约写得自相矛盾时，出图照样成功，
    /// 只是九个格子各画各的。
    pub fn issues(&self) -> Vec<String> {
        let mut out = Vec::new();

        // 同名锚点被放进两个分区 —— location-plate 的
        // 「不能在一句中把同一门同时置于左右墙」。
        let mut seen: Vec<(&str, &str)> = Vec::new();
        for a in &self.anchors {
            let name = a.name.trim();
            if name.is_empty() {
                continue;
            }
            if let Some((_, prev)) = seen.iter().find(|(n, _)| *n == name) {
                if *prev != a.zone.trim() {
                    out.push(format!(
                        "锚点「{name}」被放在两个地方：{prev} 和 {}",
                        a.zone.trim()
                    ));
                }
            } else {
                seen.push((name, a.zone.trim()));
            }
        }

        // View 里点名了一个契约里没有的锚点 —— 那就不是锚点，是随手写的装饰，
        // 换个视角它就会消失。
        for v in &self.views {
            for want in &v.visible {
                let want = want.trim();
                if want.is_empty() {
                    continue;
                }
                if !self.anchors.iter().any(|a| a.name.trim() == want) {
                    out.push(format!(
                        "视角「{}」说能看见「{want}」，但它不在锚点表里",
                        v.name.trim()
                    ));
                }
            }
        }

        // 裸写的「左 / 右」有歧义 —— 是画面左还是场内左？
        // 这一条在演员站位那边已经栽过一次，同样的坑不踩第二遍。
        for r in &self.relations {
            if bare_side(r) {
                out.push(format!("相对位置「{}」里的左/右没说清是画面还是场内", r.trim()));
            }
        }

        // 空场政策和视角描述打架。location-plate 把它列为 structural_invariant。
        if self.empty_stage == EmptyStage::Required {
            for v in &self.views {
                if v.map_line().contains("[角色:") {
                    out.push(format!("视角「{}」写了角色，但空场政策是「必须空场」", v.name.trim()));
                }
            }
        }

        // 推断项和锁死项重名 —— 说明有东西两边都登记了，
        // 分不出它到底是剧本写的还是我们编的。
        for i in &self.inferred {
            let i = i.trim();
            if !i.is_empty() && self.anchors.iter().any(|a| a.name.trim() == i) {
                out.push(format!("「{i}」同时出现在锚点表和推断项里，分不清是剧本写的还是补的"));
            }
        }

        out
    }
}

/// 句子里有没有**没限定参照系**的左/右。
///
/// 「画面左侧」「场内西侧」明确；光写「左边的门」不明确 ——
/// 换个机位它就换到右边了，而两种读法画出来都「对」。
fn bare_side(s: &str) -> bool {
    let s = s.trim();
    if !s.contains('左') && !s.contains('右') {
        return false;
    }
    const QUALIFIED: [&str; 6] = ["画面左", "画面右", "场内左", "场内右", "镜头左", "镜头右"];
    // 只要每一处左/右都带了参照系前缀，就算说清楚了。
    let total = s.matches('左').count() + s.matches('右').count();
    let named: usize = QUALIFIED.iter().map(|q| s.matches(q).count()).sum();
    named < total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn 锚(name: &str, zone: &str, size: AnchorSize) -> Anchor {
        Anchor { name: name.into(), zone: zone.into(), size, must_not_become: Vec::new() }
    }

    fn 面馆() -> SceneContract {
        SceneContract {
            shape: "东西进深的狭长一层店铺".into(),
            zones: vec!["东侧卷帘门入口".into(), "南侧客桌区".into(), "北侧柜台".into()],
            entrances: vec![Entrance {
                name: "东侧卷帘门".into(),
                connects_to: "街道".into(),
            }],
            anchors: vec![
                锚("北侧旧木柜台", "北墙", AnchorSize::WallDominant),
                锚("南墙菜单板", "南墙", AnchorSize::Small),
                锚("后厨出餐口", "柜台后方", AnchorSize::Medium),
            ],
            relations: vec!["后厨出餐口在柜台正后方".into()],
            materials: vec!["磨损木柜台".into(), "浅色旧瓷砖".into()],
            views: vec![
                SceneView {
                    name: "入口正打".into(),
                    from: "东侧卷帘门内侧".into(),
                    toward: "店堂深处".into(),
                    far: "靠北墙的旧木柜台".into(),
                    visible: vec!["北侧旧木柜台".into()],
                    light: "主光来自身后的街口，柜台区靠暖色吊灯补光".into(),
                },
                SceneView {
                    name: "柜台反打".into(),
                    from: "柜台内侧".into(),
                    toward: "东侧卷帘门".into(),
                    far: "卷帘门和门外的街".into(),
                    visible: vec!["南墙菜单板".into()],
                    light: "主光变成正面来的街口光，柜台一侧压暗".into(),
                },
            ],
            inferred: vec!["天花板的管线走向剧本没写".into()],
            no_extra: vec!["第二扇门".into(), "楼梯".into()],
            empty_stage: EmptyStage::Required,
            pano_calibration: None,
            world: None,
        }
    }

    #[test]
    fn 锁面挑最强的锚点并且带尺度() {
        let lock = 面馆().lock_line();
        assert!(lock.contains("北侧旧木柜台"), "占满墙的那个必须在：{lock}");
        assert!(lock.contains("不要缩小"), "尺度锁没发出去：{lock}");
        // 小件在只有三个名额时可以被挤掉，但这里正好三个，所以都在。
        assert!(lock.contains("入口只有 1 处"), "入口数量没说：{lock}");
        assert!(lock.contains("不要再加门"), "缺负向：{lock}");
    }

    #[test]
    fn 锁面超过三个锚点时按尺度截断() {
        let mut c = 面馆();
        c.anchors.push(锚("门边的伞桶", "东墙", AnchorSize::Small));
        c.anchors.push(锚("沿西墙的整排货架", "西墙", AnchorSize::WallDominant));
        let lock = c.lock_line();
        assert!(lock.contains("整排货架"), "大件被挤掉了：{lock}");
        assert!(!lock.contains("伞桶"), "小件不该占名额：{lock}");
    }

    #[test]
    fn 锚点的负向锁进得了锁面() {
        let mut c = 面馆();
        c.anchors[0].must_not_become = vec!["现代吧台".into()];
        assert!(c.lock_line().contains("不要画成现代吧台"), "{}", c.lock_line());
    }

    #[test]
    fn 视角描述是两两关系加视线终点() {
        let line = 面馆().views[0].map_line();
        assert!(line.contains("从东侧卷帘门内侧朝店堂深处看"), "{line}");
        assert!(line.contains("视线尽头是靠北墙的旧木柜台"), "{line}");
        assert!(line.contains("同时可见：北侧旧木柜台"), "{line}");
    }

    #[test]
    fn 视角缺项时不出现空的连接词() {
        let v = SceneView { name: "随便".into(), toward: "窗".into(), ..Default::default() };
        assert_eq!(v.map_line(), "朝窗看");
        assert!(SceneView::default().map_line().is_empty());
    }

    #[test]
    fn 契约派生出宫格视角() {
        let vps = 面馆().viewpoints();
        assert_eq!(vps.len(), 2);
        assert_eq!(vps[0].name, "入口正打");
        // 描述不能是空的 —— 空描述正是九格退化成一张图的原因。
        assert!(!vps[1].description.is_empty());
        assert!(vps[1].description.contains("卷帘门"));
    }

    #[test]
    fn 没名字的视角不出格子() {
        let mut c = 面馆();
        c.views.push(SceneView { name: "  ".into(), toward: "北".into(), ..Default::default() });
        assert_eq!(c.viewpoints().len(), 2);
    }

    #[test]
    fn 文字地图带上入口通向哪里和负向清单() {
        let block = 面馆().to_prompt_block("安和面馆");
        assert!(block.contains("场景「安和面馆」：东西进深"), "{block}");
        assert!(block.contains("东侧卷帘门通向街道"), "{block}");
        assert!(block.contains("后厨出餐口在柜台正后方"), "{block}");
        assert!(block.contains("**不要出现**：第二扇门、楼梯"), "{block}");
        assert!(block.contains("不要出现任何人物"), "空场政策没发：{block}");
    }

    #[test]
    fn 空契约要认得出来() {
        assert!(SceneContract::default().is_empty());
        assert!(!面馆().is_empty());
        let 只有关系 = SceneContract {
            relations: vec!["门正对柜台".into()],
            ..Default::default()
        };
        assert!(!只有关系.is_empty());
    }

    #[test]
    fn 空契约的锁面是空字符串() {
        // 不能返回「地理锁：。」这种壳子 —— 调用方会把它当成有内容发出去。
        assert!(SceneContract::default().lock_line().is_empty());
    }

    #[test]
    fn 自检抓同一个锚点被放在两处() {
        let mut c = 面馆();
        c.anchors.push(锚("北侧旧木柜台", "南墙", AnchorSize::WallDominant));
        let issues = c.issues();
        assert!(
            issues.iter().any(|i| i.contains("北侧旧木柜台") && i.contains("两个地方")),
            "{issues:?}"
        );
    }

    #[test]
    fn 自检抓视角点名了不存在的锚点() {
        let mut c = 面馆();
        c.views[0].visible.push("靠窗的钢琴".into());
        assert!(
            c.issues().iter().any(|i| i.contains("靠窗的钢琴") && i.contains("不在锚点表里")),
            "{:?}",
            c.issues()
        );
    }

    #[test]
    fn 自检抓裸写的左右() {
        let mut c = 面馆();
        c.relations.push("水池在左边".into());
        assert!(c.issues().iter().any(|i| i.contains("水池在左边")), "{:?}", c.issues());
    }

    #[test]
    fn 说清参照系的左右不报() {
        let mut c = 面馆();
        c.relations.push("水池在画面左侧".into());
        assert!(!c.issues().iter().any(|i| i.contains("水池")), "{:?}", c.issues());
    }

    #[test]
    fn 一句里只说清一半的左右照样报() {
        // 这是最容易漏的样子：前半句规范，后半句顺手写了「右侧」。
        assert!(bare_side("柜台在画面左侧，门在右侧"));
        assert!(!bare_side("柜台在画面左侧，门在画面右侧"));
        assert!(!bare_side("柜台靠北墙"));
    }

    #[test]
    fn 自检抓推断项和锚点重名() {
        let mut c = 面馆();
        c.inferred.push("后厨出餐口".into());
        assert!(
            c.issues().iter().any(|i| i.contains("分不清是剧本写的还是补的")),
            "{:?}",
            c.issues()
        );
    }

    #[test]
    fn 好契约自检不报任何问题() {
        assert!(面馆().issues().is_empty(), "{:?}", 面馆().issues());
    }

    /// **校准不算「有内容」。** 只有校准没有地理的契约仍然是空的 ——
    /// 它一条地理都锁不住，而 `is_empty` 判的是「值不值得发出去」。
    #[test]
    fn 只有校准的契约仍然算空() {
        let c = SceneContract {
            pano_calibration: Some(PanoCalibration { eye_height: 1.2, ground_height: 0.0, scale: 1.0 }),
            ..Default::default()
        };
        assert!(c.is_empty());
    }

    #[test]
    fn 校准能存能读并且老数据没有它() {
        let c = SceneContract {
            shape: "狭长".into(),
            pano_calibration: Some(PanoCalibration { eye_height: 1.25, ground_height: 0.0, scale: 1.5 }),
            ..Default::default()
        };
        let back: SceneContract = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        let cal = back.pano_calibration.unwrap();
        assert_eq!(cal.eye_height, 1.25);
        assert_eq!(cal.ground_height, 0.0);
        let legacy: SceneContract = serde_json::from_str(r#"{"pano_calibration":{"eye_height":1.6,"scale":1.0}}"#).unwrap();
        assert_eq!(legacy.pano_calibration.unwrap().ground_height, 0.0);
        // 老存档没有这个字段
        let old: SceneContract = serde_json::from_str(r#"{"shape":"狭长"}"#).unwrap();
        assert!(old.pano_calibration.is_none());
    }

    #[test]
    fn 序列化缺字段时能读回来() {
        // 旧存档里没有这些字段，`#[serde(default)]` 必须兜住，
        // 否则加一个字段就让所有已存资产读不出来。
        let c: SceneContract = serde_json::from_str(r#"{"shape":"狭长店铺"}"#).unwrap();
        assert_eq!(c.shape, "狭长店铺");
        assert_eq!(c.empty_stage, EmptyStage::Required);
        assert!(c.anchors.is_empty());
    }
}
