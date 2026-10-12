//! 角色的**外观规格** —— 槽位化的长相和服装。
//!
//! # 为什么不是一段自由描述
//!
//! 自由描述是我们实跑出来的这种东西：
//!
//! ```text
//! 年轻男性，身形清瘦，短发，面容带有疲惫与压抑的阴影，常穿简单的便装，
//! 25-30岁，简单朴素的日常便装
//! ```
//!
//! 三处毛病，每一处都会一路错到出图：
//!
//! 1. **该有的没有。** 没有身高、没有脸型、没有瞳色、没有发色 ——
//!    模型每出一张图就替我们决定一次，于是同一个角色每张脸都不一样。
//!    这就是「长相漂移」的来源，而它不报错。
//! 2. **「面容带有疲惫与压抑的阴影」是情绪，不是长相。** 它会被焊死在
//!    这个角色的**每一张**图上，连他高兴的那一镜也是。
//! 3. **服装写了两遍**（「常穿简单的便装」+「简单朴素的日常便装」）——
//!    自由拼接没有地方可以去重。
//!
//! 槽位化之后，「缺了什么」是**看得见**的：空槽就是空槽。
//!
//! # 空槽要写「无」，不是省略
//!
//! 抄的是 liblib 那份规格里的「无帽，无配饰」。这正是我们自己那条
//! 「负向约束是一等公民」（见 [`crate::scene_contract`]）用在衣服上：
//! 不写「无帽」的话，模型会自己发一顶帽子，而且每张图发的不是同一顶。

use serde::{Deserialize, Serialize};

/// 一个槽位：字段名、中文标签、渲染时的前缀、空着时怎么说。
struct Slot {
    /// 取值函数返回的东西前面要不要加字，比如身高要写成「身高165厘米」。
    prefix: &'static str,
    /// 空着的时候写什么。空串 = 空着就不写这一项。
    ///
    /// **只有真正会被模型脑补出来的东西才需要它。** 帽子和配饰不写就会
    /// 冒出来；脸型不写只是少一条约束，硬写「无脸型」反而是句胡话。
    when_empty: &'static str,
}

/// 锁面每个角色 ≤ 这么多字。**它发了参考图也会发**（参考图只能约束它
/// 自己画到的部分），所以必须短 —— 塞长了会把画面描述挤到上下文末尾。
pub const LOCK_CHARS: usize = 40;

/// 长相。**离开任何场景都还成立的东西** —— 不含站位、表情、动作。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CharacterLook {
    /// 身份 / 职业：「现代白领」「盛唐女官」。放第一位，它定调。
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub gender: String,
    #[serde(default)]
    pub age: String,
    /// 「165厘米」。渲染成「身高165厘米」。
    #[serde(default)]
    pub height: String,
    /// 「体型偏瘦」。
    #[serde(default)]
    pub build: String,
    /// 发型，可以带发质：「齐肩短发，发质略显干枯」。
    #[serde(default)]
    pub hair: String,
    /// 发色单独一槽 —— 和发型分开，因为换造型时发型变发色常常不变。
    #[serde(default)]
    pub hair_color: String,
    #[serde(default)]
    pub face: String,
    #[serde(default)]
    pub eyes: String,
    #[serde(default)]
    pub skin: String,
}

/// 服装。**每一层都要有话说**，包括「没有」。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CharacterOutfit {
    #[serde(default)]
    pub top: String,
    #[serde(default)]
    pub bottom: String,
    #[serde(default)]
    pub shoes: String,
    /// 空 = 渲染成「无帽」。
    #[serde(default)]
    pub headwear: String,
    /// 空 = 渲染成「无配饰」。
    #[serde(default)]
    pub accessories: String,
}

/// 一个角色的完整外观规格。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CharacterSpec {
    #[serde(default)]
    pub look: CharacterLook,
    #[serde(default)]
    pub outfit: CharacterOutfit,
}

impl CharacterLook {
    fn slots(&self) -> Vec<(&str, Slot)> {
        vec![
            (self.role.as_str(), Slot { prefix: "", when_empty: "" }),
            (self.gender.as_str(), Slot { prefix: "", when_empty: "" }),
            (self.age.as_str(), Slot { prefix: "", when_empty: "" }),
            (self.height.as_str(), Slot { prefix: "身高", when_empty: "" }),
            (self.build.as_str(), Slot { prefix: "", when_empty: "" }),
            (self.hair.as_str(), Slot { prefix: "", when_empty: "" }),
            (self.hair_color.as_str(), Slot { prefix: "", when_empty: "" }),
            (self.face.as_str(), Slot { prefix: "", when_empty: "" }),
            (self.eyes.as_str(), Slot { prefix: "", when_empty: "" }),
            (self.skin.as_str(), Slot { prefix: "", when_empty: "" }),
        ]
    }

    /// **模型把槽位名原样填回来等于没填。**
    ///
    /// 2026-09-26 实测（ornith-1.5-35b-a3b-uncensored，从人物小传提取）：原文没写程远的眼睛和肤色，
    /// 模型没有推一个值，而是把 `"eyes": "眼睛"`、`"skin": "肤色"` 交了回来 ——
    /// 渲染出来就是描述末尾一句光秃秃的「眼睛，肤色」，还会跟着进出图提示词。
    /// 清空之后按空槽处理（界面上会显示缺什么）。
    pub fn drop_label_echoes(&mut self) {
        let clear = |v: &mut String, labels: &[&str]| {
            if labels.contains(&v.trim()) {
                v.clear();
            }
        };
        clear(&mut self.role, &["身份"]);
        clear(&mut self.gender, &["性别"]);
        clear(&mut self.age, &["年龄"]);
        clear(&mut self.height, &["身高"]);
        clear(&mut self.build, &["体型", "身材"]);
        clear(&mut self.hair, &["发型", "头发"]);
        clear(&mut self.hair_color, &["发色", "头发颜色"]);
        clear(&mut self.face, &["脸型", "脸"]);
        clear(&mut self.eyes, &["眼睛", "眼", "瞳色", "瞳孔"]);
        clear(&mut self.skin, &["肤色", "皮肤"]);
    }

    /// 哪些槽还空着。**给界面用** —— 缺的东西要看得见，
    /// 而不是等出图之后才发现每张脸都不一样。
    pub fn missing(&self) -> Vec<&'static str> {
        let named: [(&str, &'static str); 10] = [
            (self.role.as_str(), "身份"),
            (self.gender.as_str(), "性别"),
            (self.age.as_str(), "年龄"),
            (self.height.as_str(), "身高"),
            (self.build.as_str(), "体型"),
            (self.hair.as_str(), "发型"),
            (self.hair_color.as_str(), "发色"),
            (self.face.as_str(), "脸型"),
            (self.eyes.as_str(), "瞳色"),
            (self.skin.as_str(), "肤色"),
        ];
        named
            .iter()
            .filter(|(v, _)| v.trim().is_empty())
            .map(|(_, label)| *label)
            .collect()
    }
}

impl CharacterOutfit {
    /// **这个角色是不是「穿衣服的」。**
    ///
    /// 上衣、下装、鞋一件都没有 = 它不是个穿衣服的人：
    /// 手机屏幕上的卡通 AI 头像、一团光、一只猫、一个图标。
    fn wears_clothes(&self) -> bool {
        [&self.top, &self.bottom, &self.shoes].iter().any(|v| !v.trim().is_empty())
    }

    fn slots(&self) -> Vec<(&str, Slot)> {
        // **不穿衣服的角色不渲染「无帽，无配饰」。**
        //
        // 那两句是给**人**用的：不说「无帽」模型就自己发一顶，而且每张图不是同一顶。
        // 但放在「蓝色卡通 AI 助理头像」上，它读起来就是
        // 「一个没戴帽子、没戴配饰的人」—— 于是出来一个真人
        // （2026-09-19 用户在 E05 的小青身上撞到）。
        //
        // 判据是「有没有衣物」，不是「是不是人」—— 后者没法从字段判断，
        // 而前者正好是同一件事的可观测形式。
        let clothed = self.wears_clothes();
        let when_empty = |s: &'static str| if clothed { s } else { "" };
        vec![
            (self.top.as_str(), Slot { prefix: "上衣", when_empty: "" }),
            (self.bottom.as_str(), Slot { prefix: "下装", when_empty: "" }),
            (self.shoes.as_str(), Slot { prefix: "", when_empty: "" }),
            // **这两条是这个模块存在的一半理由。**
            // 不说「无帽」，模型就自己发一顶，而且每张图不是同一顶。
            (self.headwear.as_str(), Slot { prefix: "", when_empty: when_empty("无帽") }),
            (self.accessories.as_str(), Slot { prefix: "", when_empty: when_empty("无配饰") }),
        ]
    }
}

/// 把槽位连成一句。空槽按 `when_empty` 处理，没有 `when_empty` 的直接跳过。
fn join(slots: Vec<(&str, Slot)>) -> String {
    let parts: Vec<String> = slots
        .into_iter()
        .filter_map(|(v, s)| {
            let v = v.trim();
            if v.is_empty() {
                (!s.when_empty.is_empty()).then(|| s.when_empty.to_string())
            } else {
                Some(format!("{}{}", s.prefix, v))
            }
        })
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("{}。", parts.join("，"))
    }
}

impl CharacterSpec {
    /// 「描述」那一行。
    pub fn look_line(&self) -> String {
        join(self.look.slots())
    }

    /// 「服装」那一行。
    pub fn outfit_line(&self) -> String {
        join(self.outfit.slots())
    }

    /// 一个槽都没填。
    ///
    /// **判据是槽位，不是渲染结果。** 第一版写的是
    /// `look_line().is_empty() && outfit_line().is_empty()`，被自己的测试
    /// 打回来了：服装行永远非空（空着也会渲染出「无帽，无配饰」），
    /// 于是一个全空的规格会报告「非空」—— 界面就显示「已有外观规格」，
    /// 而它什么都锁不住。这正是这个仓库最典型的失败形状。
    pub fn is_empty(&self) -> bool {
        self.look
            .slots()
            .into_iter()
            .chain(self.outfit.slots())
            .all(|(v, _)| v.trim().is_empty())
    }

    /// 存进资产 `description` 的那一段：描述 + 服装 + 特征。
    ///
    /// **特征从身份锚点来，不另存一份** —— `marks` 已经是「判别性强的
    /// 那几条」，再开一个「特征」字段就是两个所有者，迟早对不上。
    /// **逐镜重复用的身份锚点** —— 一行，短。
    ///
    /// # 为什么不能直接用 `description_block`
    ///
    /// 那一份是给设定卡出图用的，整段能到两三百字。而这一句要
    /// **每一镜都重复一遍**：一镜三个角色就是三份，塞进去会把画面描述
    /// 挤到上下文末尾 —— 那是已知会让出片跑偏的病。
    ///
    /// # 为什么要逐镜重复
    ///
    /// OpenMontage 的结论（`seedance-prompting.md`）：
    /// 「同一个角色」「她」这类指代**不起作用**，模型把每个镜头当成
    /// 从零说的；要把 3~6 个区分性外观特征**一字不差地重复**。
    /// 我们原来只发「保持 林婉清、皇帝 的外观一致」—— 只点名字不给特征，
    /// 对模型基本是空话。真正在起作用的是参考图，而参考图挂不上时
    /// （标签没打、资产没入库）就完全没有身份信息了。
    ///
    /// # 挑哪几槽
    ///
    /// **区分性最强的那几个**，不是全部：身份/年龄/体型/发型发色/上衣。
    /// 眼睛、皮肤、鞋子这些在中景以外根本看不见，写了只占字数。
    pub fn anchor_line(&self, limit: usize) -> String {
        let l = &self.look;
        let hair = match (l.hair.trim(), l.hair_color.trim()) {
            ("", c) => c.to_string(),
            (h, "") => h.to_string(),
            (h, c) => format!("{c}{h}"),
        };
        let parts: Vec<&str> = [
            l.role.trim(),
            l.age.trim(),
            l.build.trim(),
            hair.as_str(),
            self.outfit.top.trim(),
        ]
        .into_iter()
        .filter(|v| !v.is_empty())
        .collect();
        let joined = parts.join("，");
        // **按字符截断，不按字节** —— 中文一个字三字节，按字节切会切出半个字。
        if joined.chars().count() <= limit {
            joined
        } else {
            joined.chars().take(limit).collect()
        }
    }

    pub fn description_block(&self, marks: &[String]) -> String {
        let mut out = String::new();
        let look = self.look_line();
        if !look.is_empty() {
            out.push_str(&format!("描述：{look}"));
        }
        let outfit = self.outfit_line();
        if !outfit.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("服装：{outfit}"));
        }
        let marks: Vec<&str> = marks.iter().map(|m| m.trim()).filter(|m| !m.is_empty()).collect();
        if !marks.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("特征：{}。", marks.join("，")));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn 槽位名原样填回来的当空() {
        let mut l = CharacterLook { eyes: "眼睛".into(), skin: " 肤色 ".into(), face: "长方脸".into(), ..Default::default() };
        l.drop_label_echoes();
        assert!(l.eyes.is_empty() && l.skin.is_empty());
        assert_eq!(l.face, "长方脸", "真的值被清掉了");
    }

    use super::*;

    /// **不穿衣服的角色不渲染「无帽，无配饰」。**
    ///
    /// 那两句是给人用的（不说「无帽」模型就自己发一顶）。放在
    /// 「蓝色卡通 AI 助理头像」上，它读起来就是「一个没戴帽子的人」——
    /// 于是出来一个真人（2026-09-19 用户在 E05 的小青身上撞到）。
    #[test]
    fn 没有衣物时不写无帽无配饰() {
        let 头像 = CharacterOutfit::default();
        assert!(!头像.wears_clothes());
        let s = join(头像.slots());
        assert!(s.is_empty(), "不穿衣服的角色不该有服装句：{s}");
    }

    /// 穿衣服的照旧 —— **这两句是这个模块存在的一半理由**，不能一起丢掉。
    #[test]
    fn 穿衣服的照旧写无帽无配饰() {
        let mut o = CharacterOutfit::default();
        o.top = "白衬衫".into();
        assert!(o.wears_clothes());
        let s = join(o.slots());
        assert!(s.contains("无帽") && s.contains("无配饰"), "{s}");
    }

    fn 林婉清() -> CharacterSpec {
        CharacterSpec {
            look: CharacterLook {
                role: "现代白领".into(),
                gender: "女性".into(),
                age: "28岁".into(),
                height: "165厘米".into(),
                build: "体型偏瘦".into(),
                hair: "齐肩短发，发质略显干枯".into(),
                hair_color: "黑发".into(),
                face: "瓜子脸".into(),
                eyes: "黑瞳".into(),
                skin: "肤色苍白".into(),
            },
            outfit: CharacterOutfit {
                top: "白色微褶皱衬衫".into(),
                bottom: "黑色包臀裙".into(),
                shoes: "黑色皮鞋".into(),
                headwear: String::new(),
                accessories: String::new(),
            },
        }
    }

    /// 对着 liblib 那份规格逐字核对 —— 这是抄作业，抄错了就白抄。
    #[test]
    fn 渲染出来和参考样本一字不差() {
        let s = 林婉清();
        assert_eq!(
            s.look_line(),
            "现代白领，女性，28岁，身高165厘米，体型偏瘦，齐肩短发，发质略显干枯，黑发，瓜子脸，黑瞳，肤色苍白。"
        );
        assert_eq!(s.outfit_line(), "上衣白色微褶皱衬衫，下装黑色包臀裙，黑色皮鞋，无帽，无配饰。");
    }

    /// **空的帽子和配饰要变成「无帽」「无配饰」。**
    ///
    /// 不说的话模型自己发一顶，而且每张图发的不是同一顶 ——
    /// 这是整个模块最要紧的一条。
    #[test]
    fn 没戴帽子要明说没戴() {
        let s = 林婉清();
        assert!(s.outfit_line().contains("无帽"), "{}", s.outfit_line());
        assert!(s.outfit_line().contains("无配饰"), "{}", s.outfit_line());
    }

    /// 戴了就写戴的那个，不能还说「无帽」。
    #[test]
    fn 戴了帽子就不说无帽() {
        let mut s = 林婉清();
        s.outfit.headwear = "黑色幞头".into();
        assert!(s.outfit_line().contains("黑色幞头"), "{}", s.outfit_line());
        assert!(!s.outfit_line().contains("无帽"), "{}", s.outfit_line());
    }

    /// 长相的空槽**不编话**。「无脸型」是句胡话，只会浪费提示词。
    #[test]
    fn 长相的空槽直接跳过不编话() {
        let mut s = 林婉清();
        s.look.face = String::new();
        let line = s.look_line();
        assert!(!line.contains("无脸"), "{line}");
        assert!(!line.contains("，，"), "连出了空槽：{line}");
        assert!(line.contains("黑瞳"), "{line}");
    }

    /// 缺了什么要**报得出来** —— 缺的槽不报，就只能等出图之后
    /// 发现每张脸都不一样。
    #[test]
    fn 说得出还缺哪几项() {
        let mut s = 林婉清();
        s.look.height = String::new();
        s.look.eyes = "  ".into();
        assert_eq!(s.look.missing(), vec!["身高", "瞳色"]);
        assert!(林婉清().look.missing().is_empty(), "填满了却说缺");
    }

    /// **空规格要报告成空的。**
    ///
    /// 陷阱在于**只要填了一件衣物**，服装行就会带上「无帽，无配饰。」——
    /// 拿渲染结果当判据的话（第一版就是），那半句会让空壳看着像
    /// 「已有外观规格」，而它什么都锁不住。
    ///
    /// 2026-09-19 补：一件衣物都没填时**不再**渲染那半句
    /// （那是给穿衣服的人用的，放在「卡通 AI 头像」上会出一个真人）。
    /// 所以这里分两种情况测。
    #[test]
    fn 空规格要报告成空的() {
        let s = CharacterSpec::default();
        assert_eq!(s.look_line(), "");
        assert_eq!(s.outfit_line(), "", "一件衣物都没有时不该凭空写「无帽」");
        assert!(s.is_empty());

        // 填了一件衣物之后，陷阱回来了：服装行里多出「无帽，无配饰」
        let mut s2 = CharacterSpec::default();
        s2.outfit.top = "白衬衫".into();
        assert!(s2.outfit_line().contains("无帽，无配饰"), "{}", s2.outfit_line());
        assert!(!s2.is_empty());
    }

    /// 填了任意一个槽就不算空。
    #[test]
    fn 填了一个槽就不算空() {
        let mut s = CharacterSpec::default();
        s.outfit.headwear = "黑色幞头".into();
        assert!(!s.is_empty());
        let mut s2 = CharacterSpec::default();
        s2.look.eyes = "黑瞳".into();
        assert!(!s2.is_empty());
    }

    /// 存进资产的那一段：三行，特征来自身份锚点。
    #[test]
    fn 描述块是描述加服装加特征() {
        let s = 林婉清();
        let block = s.description_block(&["黑眼圈明显".into(), "眼角微垂".into()]);
        let lines: Vec<&str> = block.lines().collect();
        assert_eq!(lines.len(), 3, "{block}");
        assert!(lines[0].starts_with("描述：现代白领"), "{block}");
        assert!(lines[1].starts_with("服装：上衣白色"), "{block}");
        assert_eq!(lines[2], "特征：黑眼圈明显，眼角微垂。");
    }

    /// 没有锚点时不留一行空的「特征：」。
    #[test]
    fn 没有特征就不写特征那一行() {
        let block = 林婉清().description_block(&[String::new(), "  ".into()]);
        assert!(!block.contains("特征"), "{block}");
    }
}
