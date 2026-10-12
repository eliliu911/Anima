# 块状提示词构建法（SDXL / RealVisXL）

借鉴 Seedance skill 的 prompt-blocks 思路和 BFL 的 json-structured-prompting，
按本机的 SDXL 链路重写。**顺序有意义** —— CLIP 的注意力对靠前的 token 更强，
所以主体和构图在前，风格和画质在后。

```
[主体]  [主体细节]  [动作/姿态]  [环境]  [构图/景别]  [光线]  [镜头/介质]  [画质]
```

| 块 | 作用 | 例 | 在 SDXL 上的有效性 |
|:---|:---|:---|:---|
| 主体 | 谁/什么 | `a 20-year-old Asian woman` | 强 |
| 主体细节 | 发型、服装、体态 | `high ponytail, white cropped top` | 强 |
| 动作/姿态 | 在做什么 | `walking, arms raised` | **弱 —— 复杂姿势用 `pose_image`** |
| 环境 | 在哪 | `bustling city street at dusk` | 强 |
| 构图/景别 | 怎么框 | `full body shot, centered` | 中 |
| 光线 | 见 lighting.md | `soft rim light from behind` | 强 |
| 镜头/介质 | 见 lens.md | `85mm portrait lens, shallow depth of field` | 强 |
| 画质 | 收尾 | `photorealistic, natural skin texture` | 中 |

## 三条本机实测的规则

1. **姿势块基本不起作用。** 把树式写到「点名 vrksasana + 哪条腿站 + 脚放哪 +
   exactly two legs」，4 个种子里 3 张是别的动作。复杂姿势必须走 `pose_image`。
2. **画质块要具体。** `high quality` 几乎无效；`natural skin texture` /
   `raw photo` / `studio lighting` 有效。抽象形容词在 CLIP 里几乎没有落点。
3. **中文块直接丢失。** 整段中文进 SDXL 只会剩一个模糊的主体概念，
   发型服装背景构图全丢（本机对照实测）。先自己译成英文。

## 视频链路不一样

MiniMax H3 的编码器是 Qwen3-VL，**中文原生，而且句子越完整越好** ——
它不是标签模型。所以视频提示词写成**自然语言的镜头描述**，不要写成逗号分隔的标签：

```
差：少女, 白色上衣, 街道, 跟拍, 电影感
好：一位扎高马尾的年轻女性走在傍晚的城市街道上，镜头在她侧方平行跟随，
    背景行人虚化，夕阳从她身后打来形成轮廓光
```

台词直接写进提示词，H3 会生成同步中文配音和对应口型（本机转写验证过）。
