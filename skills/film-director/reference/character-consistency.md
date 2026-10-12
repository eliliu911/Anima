# 角色一致性

方法论借鉴 `nigo-studio/ai-film-skills` 的 character-consistency，
执行手段换成本机已有的工具参数。

## 核心论点：细节即一致性

**模糊的提示词生成通用脸，而通用脸无法保持一致** —— 因为下一次生成没有可抓的
特征。所以一致性不是"每次写同样的提示词"，而是"先把角色定义到足够具体，
再把这个定义固化成可复用的资产"。

## 三步

### 1. 把每个属性扩写成句子，不是标签

```
差：20岁，亚洲女性，黑发，白色上衣
好：a 20-year-old Asian woman with a high ponytail of dark brown hair,
    fine flyaway strands at the temples, almond eyes with a slight upward tilt,
    a small mole below the left eye, wearing a white ribbed cropped tee
```

有落点的细节（痣的位置、发丝、眼型）才是下一次生成能抓住的东西。

### 2. 固化成设定卡

```
generate_image(layout: "character_sheet", prompt: <上面那段>)
```

一次扩散出整排多视图（正/侧/背），1536×1024。版式触发词和版式 LoRA 由代码自动加，
提示词里**只描述角色本身，不要写版式词**。

### 3. 每个镜头都挂这张卡

```
animate_image(mode: "reference", image_path: <设定卡>, prompt: <镜头描述>)
```

本机实测：一张整合的设定卡 ≈ 三张分开的视图（264s vs 250s，效果相当），
所以一张卡就够。

## seed 的作用和边界

- 工具返回里报的 `seed` 原样传回 → 保持人物与构图
- **换新随机 seed = 换一个人**，不是"微调"
- 但 seed 只在**同一条链路、同一组参数**下有意义；改分辨率或换引擎后不成立

## 已知限制

设定卡整排图里，每张脸只有 40~46 px。**提示词救不了这个** —— 需要单独的脸部
特写时，用 `generate_image` 单独出一张近景，而不是指望从整排图里放大。
