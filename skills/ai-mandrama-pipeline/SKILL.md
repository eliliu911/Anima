---
name: ai-mandrama-pipeline
description: 帮助用户用 Dreamina CLI + edge-tts + ffmpeg 端到端制作中文 AI 漫剧/短剧（日漫赛璐璐风、1-3 分钟、横版+竖版双端、配音+字幕+BGM+片头片尾完整成片），用一套"角色三视图锚定 + 14 镜头串行 image2video + 后台 polling 自动化 + ffmpeg master 拼接"流水线替代"text2image 全程导致角色漂移 / 视频任务并发撞限流 / libass 中文字幕变方块"的常见坑。当用户描述"我有剧本一卡和分镜表想做 AI 漫剧 / 把短剧剧本做成动态漫剧 / 鱼子酱 AI 漫剧课程 SOP 实操 / 一集 14 镜头怎么从分镜到成片 / 我做 AI 短剧角色每帧脸都不一样 / 我视频任务卡在 dreamina 队列 / 我做完角色三视图下一步怎么走 / 用 ffmpeg 拼成中文短剧字幕变方块 / 我没人值守想让它自动跑完一整集 / 我想出横版+9:16 竖版两个版本"时使用；当用户问"什么是漫剧 SOP 9 阶段 / 项目目录结构 00-09 / 角色三视图怎么生 / 哪些镜头必须 image2image / dreamina i2i 能不能并行 / image2video 服务端并发上限 / seedance2.0fast 怎么选 / 后台 polling watchdog 怎么写 / ffmpeg drawtext 中文字幕怎么避方块 / amix 老版 normalize 不识别怎么办 / BGM 怎么用 ffmpeg sine 合成占位 / 5.0 模型长 prompt 触发 final generation failed / dreamina credit 失败也扣费 / submit_id 被 stderr 污染 / 9:16 竖版怎么从 16:9 重排 / 片头片尾 4s 标题卡怎么做"时使用；当用户在做"中文 AI 短剧 / AI 漫剧 / 末世题材 / 爽文改编 / 日漫赛璐璐风短视频 / 动态漫"且要走"剧本→分镜→角色→生图→生视频→配音→拼接→成片"完整流程时使用。提供：1) **SOP 9 阶段流程图**（剧本一卡/分镜表/角色介绍/资产库/单镜头生图/单镜头生视频/首尾帧/剪辑/成片）+ 项目目录结构规范；2) **角色三视图迭代心法**（写实→二次元→治愈系三版血泪经验 + 男主/女主/配角 prompt 关键词速查）；3) **image2image 锚定规则**（有人物镜头必须用 i2i + 三视图作参考，t2i 只用于无人物场景）；4) **Dreamina CLI 10 项关键踩坑速查**（i2i 不能并行 / 视频并发=1 / 5.0 长 prompt 审查 / submit_id 严格 UUID 正则 / tail 截 url / credit 失败也扣费）；5) **后台 polling + watchdog 无人值守架构**（python 串行提交 + 30s 查询 + 80 次超时 + watchdog 自动调 master）；6) **ffmpeg master 5 步拼接 pipeline**（标准化 clip + 拼接 + 字幕烧入 + 20 通道 amix 混音 + 9:16 竖版）；7) **drawtext 中文字幕方案**（textfile= 替 text= 避转义 / enable=between 控制时段 / borderw+shadow 强对比 / STHeiti Medium.ttc 路径转义 / 角色色彩字号配方）；8) **末世招队员第一集完整范例**（14 镜头 + 1m02s 横版+竖版 + 390 credit / 5 小时复盘）。**与同源姐妹的边界**：本 Skill 是国内中文短剧场景的端到端工作流。`jimeng-multishot-replicate` 是 ins 投放 niche 黑白线稿 17 帧场景；`ffmpeg-reels-pipeline` 是 ins 9:16 18s 英文 reels；本 Skill 是中文漫剧 1-3 分钟 16:9+9:16 双端完整成片（有片头片尾+配音+字幕+BGM）。配音环节调用 `edge-tts-chinese-roleplay`（中文角色化 SSML TTS 子流程）。**不适用于**：单 GIF/海报生成（应直接用 dreamina text2image）、ins 黑白线稿复刻（用 jimeng-multishot-replicate）、真人短剧/混剪（不在 AI 生成范畴）。
name_zh: "AI 漫剧流水线（mandrama）"
marker_zh: "漫剧"
description_zh: "Dreamina + edge-tts + ffmpeg 的中文漫剧端到端流程（云端工具链，和本机链路不同）。"
source: "https://github.com/cyuanxv/ai-mandrama-skills @ 0142fc1（MIT）。Anima 只在 frontmatter 加了中文名/标记/说明/来源四个字段，正文未改。"
---

# AI 漫剧端到端制作流水线（中文）

## 何时使用本 Skill

- 用户有剧本一卡 + 分镜表，要把它做成中文 AI 漫剧/短剧
- 用户在跑"鱼子酱 AI 漫剧"课程 SOP 实操，要 Dreamina + ffmpeg 全套
- 用户问"分镜→生图→生视频→配音→拼接"任意环节的踩坑/最佳实践
- 用户要做 1-3 分钟横版 + 9:16 竖版双端成片
- 用户睡觉前要"无人值守自动跑完一整集"
- 用户用 dreamina 视频任务遇到队列/并发限制
- 用户用 ffmpeg drawtext 烧中文字幕遇到方块/转义问题

## 核心流程（SOP 9 阶段）

### 阶段 1-3：素材准备

| 阶段 | 文件 | 说明 |
|---|---|---|
| 1. 剧本一卡 | `00_剧本/一卡_v1.docx` | 动态漫剧本，每镜头描述场景+人物+动作+对白 |
| 2. 分镜表 | `01_分镜表/第一集_分镜表_v1.xlsx` | 9 列结构（镜头号/场景/人物/动作/提示词/运镜/对白/对应对白/备注）|
| 3. 角色介绍 | `02_核心场景与角色/第一集_核心场景与角色.docx` | 角色设定参考（外貌/服装/性格）|

**项目目录强制规范**：
```
~/Downloads/AI短剧_<剧名>/
├── 00_剧本/
├── 01_分镜表/
├── 02_核心场景与角色/
├── 03_资产库/第一集/角色/    ← 三视图存这
├── 04_单镜头生图/第一集/      ← 14 张分镜单图
├── 05_单镜头生视频/第一集/    ← 13-14 段 mp4
├── 06_首尾帧素材/第一集/      ← 可选
├── 08_成片/                  ← 最终交付
└── README_制作过程.md
```

### 阶段 4：角色资产库（三视图）— 关键迭代

**风格选型决策**：
- ❌ 写实风（"8K高清 + 电影级质感 + 写实风格"）→ 容易让爽文角色看起来太"棚拍模特"
- ✅ **日漫赛璐璐二次元**（"日漫赛璐璐平涂上色，类似《全职高手》《魔道祖师》《天官赐福》乙游风格"）→ 漫剧标配
- ⚠️ 治愈系日漫（《夏目友人帐》风）→ 太软，爽文不适用

**用户反馈"明媚温柔"≠ 切换风格基调**，而是**相对方向微调**——保持二次元基调 + prompt 加入"温柔/明亮/带笑意"。**经验教训：不要矫枉过正**。

**角色 prompt 速查**（详细模板见 `reference.md`）：

| 角色类型 | 关键词 |
|---|---|
| **爽文男主** | 桃花眼略上挑、剑眉斜飞、薄唇微抿带邪魅、黑色微长碎发气流感、慵懒锋利眼神 |
| **F 级女主/可怜系** | 纤瘦娇小、黑色长发披肩微卷、樱桃唇、楚楚可怜被欺负、破旧但本身好看的反差 |
| **配角性感美女** | 沙漏型胸大腰细、浅棕色大波浪卷长发、温柔明媚大姐姐 |
| **反派/粗暴男** | 身材魁梧、满脸横肉胡渣、黑色破皮衣 |

**命令模板**：
```bash
dreamina text2image \
  --prompt="高质量日韩漫画动漫人物设定稿，<角色>三视图：正面/侧面/背面横向并排，纯白背景，无任何水印文字。<角色五官身材服装详细描述>。日漫赛璐璐平涂上色，干净流畅线条，精致五官刻画，类似《全职高手》乙游风格。三视图保持发型五官身材服装严格一致，仅角度不同。8K高清，无文字，无水印。" \
  --ratio=16:9 --resolution_type=2k --model_version=5.0 --poll=90
```

**同角色换装**（如男主庄园造型 vs 集市造型）**用 image2image 拿三视图当锚定**，保证人脸一致性：
```bash
dreamina image2image \
  --images="$角色_庄园造型_三视图.png" \
  --prompt="保持参考图人物的脸/发型/五官/身材/画风/纯白背景/三视图布局完全一致，只把服装换成<新服装>" \
  --ratio=16:9 --resolution_type=2k --model_version=5.0 --poll=120
```

### 阶段 5：单镜头生图（14 张）

**核心心法**：**有人物的镜头必须用 image2image + 角色三视图作锚定**，否则角色每帧脸都不一样。

**判定规则**：

| 镜头特征 | 命令 | 锚定 |
|---|---|---|
| 无人物（废墟/环境/系统面板）| `text2image` | — |
| 单人物近景/中景 | `image2image` | 该角色三视图 |
| 双人物互动 | `image2image` | 两个角色三视图（多图作 `--images`）|
| 多人物群像 | `image2image` | 主角三视图 + prompt 描述其他人 |

**image2image 多参考图命令**：
```bash
dreamina image2image \
  --images="$陆明.png,$宁曦.png" \
  --prompt="日漫赛璐璐画风<场景>。画面<位置>：<角色A>(参考第一张图人物)<动作>。<位置>：<角色B>(参考第二张图人物)<动作>。<背景>。<光影色调>。日漫平涂线条干净。<景别>。8K高清无文字无水印。两个角色严格遵循参考图人物。" \
  --ratio=16:9 --resolution_type=2k --model_version=5.0 --poll=120
```

### 阶段 6：单镜头生视频（14 段）

**模型选型**：`seedance2.0fast` 720p 5s/段（maestro VIP 推荐），每段 **~25-30 credit**。

**核心限制：服务端单用户视频并发上限 = 1**

每段必须串行排队：
- 队列等待 **~10-12 分钟**（队列长度 ~600，每 30 秒消耗 30 位）
- 实际生成 **~30-60 秒**
- 单段总耗 **~12-15 分钟**
- 14 段共 **~3 小时**

**后台 polling + watchdog 无人值守架构**（详细代码见 `reference.md`）：

```
i2v_polling.py  ──串行提交+30s 间隔 query+80 次超时──> 14 段 mp4
       │
       ▼ (完成后)
watchdog_v4.sh  ──检测 ALL DONE──> 调起 master_v4.sh 自动拼接
```

**关键提交+查询函数**（Python，**严格 UUID 正则防 stderr 污染**）：
```python
import re, subprocess

def extract_uuid(text):
    m = re.search(r'"submit_id"\s*:\s*"([a-f0-9-]{36})"', text)
    return m.group(1) if m else None

def submit_one(image_path, prompt):
    for retry in range(3):
        r = subprocess.run([
            'dreamina', 'image2video',
            '--image', image_path,
            '--prompt', prompt,
            '--duration', '5',
            '--video_resolution', '720p',
            '--model_version', 'seedance2.0fast',
            '--poll', '10'
        ], capture_output=True, text=True, timeout=120)
        sid = extract_uuid(r.stdout + r.stderr)
        if sid and '"fail"' not in r.stdout:
            return sid
        time.sleep(60)  # 重试间隔
    return None

def wait_and_download(sid, label, out_path, max_polls=80):
    for i in range(max_polls):
        out = subprocess.run(['dreamina', 'query_result', '--submit_id', sid],
                            capture_output=True, text=True).stdout
        url_m = re.search(r'"video_url"\s*:\s*"([^"]+)"', out)
        if url_m:
            urllib.request.urlretrieve(url_m.group(1), out_path)
            return out_path
        time.sleep(30)
    return None
```

### 阶段 7：配音

**配音环节调用姐妹 Skill `edge-tts-chinese-roleplay`**——15 段中文对白 + 角色化声线分配 + SSML rate/pitch 情绪精调，输出 15 个 mp3。

详见 `edge-tts-chinese-roleplay` skill。

### 阶段 8：BGM + SFX 合成（占位级）

**ffmpeg sine 合成 ambient BGM**（替代真实音乐，占位用，后续可在剪映里换专业曲）：

```bash
# 末世低吟 13s — C2(65Hz) + G2(98Hz) + 低八度 49Hz
ffmpeg -f lavfi -i "sine=f=65:d=13" -f lavfi -i "sine=f=98:d=13" -f lavfi -i "sine=f=49:d=13" \
  -filter_complex "[0][1][2]amix=3,tremolo=f=0.4:d=0.5,afade=t=in:st=0:d=2,afade=t=out:st=11:d=2,volume=2.5" \
  -ar 44100 末世.wav

# 闪回轻音 12s — C4(262)+E4(330)+G4(196 八度下) + vibrato
# 集市紧张 25s — 多频段(147+220+110+294)叠加 + tremolo=f=2
# 系统科技 7s — 高频(440+659+880)sine 跳动 + tremolo=f=8
# 4 段 concat 成 60s
```

**SFX 4 个**：
| 音效 | 合成方式 |
|---|---|
| thunder（闪电）| `anoisesrc=c=brown:a=0.8` + `sine=f=60` 混合 + 短淡出 |
| woosh（瞬移）| `sine=f=2000` + `asetrate=44100*0.5,aresample=44100` 变速 sweep |
| ding（系统弹窗）| `sine=f=880` + `sine=f=1320` 双音 + apad |
| thud/kick（猝死/踹倒）| `sine=f=80~100` + `anoisesrc=c=pink/brown` 混合 |

**⚠️ ffmpeg 老版本（如 N-92585 / 2018）amix `normalize=` 参数不识别**——删掉该参数 + `volume=N` 手动补回自动衰减。

### 阶段 9：成片拼接（master 脚本）

**5 步 pipeline**（完整 176 行 master 脚本见 `reference.md`）：

1. **每镜头标准化 clip**：720p 30fps + 调色（`eq=contrast=1.05:saturation=1.12:gamma=0.98,vignette=PI/5`）+ fade in/out 0.3s
2. **拼接**：片头 4s + 14 镜头 + 片尾 4s ≈ **65s**（concat demuxer）
3. **drawtext 烧字幕**（关键，见下方踩坑）
4. **混音**：BGM(0.18 vol) + 15 段 TTS(每段 adelay+volume=14) + 4 个 SFX = **20 输入流 amix**
5. **9:16 竖版**：`scale=720:-1,pad=720:1280:0:(1280-ih)/2:black`

**片头/片尾 4s 标题卡**：
```bash
FONT='/System/Library/Fonts/STHeiti Medium.ttc'
ffmpeg -f lavfi -i "color=c=#0a0a14:s=1280x720:d=4" \
  -vf "drawtext=fontfile=${FONT// /\\ }:fontsize=72:fontcolor=white:x=(w-tw)/2:y=h/2-80:text='剧名', \
       drawtext=fontfile=${FONT// /\\ }:fontsize=56:fontcolor=#FFD700:x=(w-tw)/2:y=h/2+20:text='副标题', \
       drawtext=fontfile=${FONT// /\\ }:fontsize=32:fontcolor=#888888:x=(w-tw)/2:y=h/2+100:text='第X集 子标题', \
       fade=t=in:st=0:d=0.8,fade=t=out:st=3.2:d=0.8" \
  -c:v libx264 -preset fast -crf 20 -pix_fmt yuv420p -r 30 -an intro.mp4
```

## 中文字幕渲染（重大踩坑 v4→v4.1）

**❌ libass `subtitles=` 滤镜烧 ASS 字幕在 macOS 上中文变方块 □□□**

原因：libass 找不到 ASS 里指定的 STHeiti 字体（macOS 的 fontconfig 未注册 ttc）。

**✅ 修复方案：改用 `drawtext` + textfile + 直接字体路径**

```bash
FONT='/System/Library/Fonts/STHeiti Medium.ttc'

# 每段对白写到独立 textfile（避中文逗号/感叹号转义噩梦）
cat > /tmp/subs/01.txt <<EOF
陆明：不准训练哈，在我这你只管摸鱼躺平！
EOF

# 用 drawtext 链式 + enable=between 控制每段显示时段
ffmpeg -i raw.mp4 -filter_complex "
[0:v]drawtext=fontfile=${FONT// /\\ }:fontsize=36:fontcolor=#FFDD00:\
     textfile=/tmp/subs/01.txt:enable='between(t,9.5,14)':\
     x=(w-text_w)/2:y=h-100:\
     borderw=2:bordercolor=black:shadowcolor=black:shadowx=2:shadowy=2[v1];
[v1]drawtext=...:textfile=/tmp/subs/02.txt:enable='between(t,14.5,17)'[v2];
... (15 段链式)
" out.mp4
```

**关键技术点**：
- **`textfile=` 替代 `text=`** → 避中文标点转义
- **`enable='between(t,start,end)'`** → 控制时段
- **`borderw=2:bordercolor=black + shadowx/y=2`** → 强对比保证手机可读
- **字体路径含空格用 `${FONT// /\\ }`** → bash 转义

**字幕样式按角色色彩区分**：
| 角色 | 颜色 | 字号 |
|---|---|---|
| 旁白 | 白 #FFFFFF | 34pt |
| **主角** | **黄 #FFDD00** | 36pt（关键台词放大到 60pt）|
| 系统 | 绿 #00FF00 | 36pt |
| 反派 | 红 #FF1515 | 36pt |
| 配角 | 灰 #DDDDDD | 32pt |

## Dreamina CLI 10 项关键踩坑速查

| # | 踩坑 | 修复 |
|---|---|---|
| 1 | **i2i 并行 silent fail** | 严格串行 |
| 2 | **服务端单用户视频并发=1** | 后台 polling + watchdog 串行队列 |
| 3 | **bash submit_id 被 stderr 污染** | 严格 UUID 正则 `[a-f0-9]{8}-[a-f0-9]{4}-...` |
| 4 | **macOS say 90% 中文 voice 空音** | 切 edge-tts（见姐妹 skill）|
| 5 | **ffmpeg 老版 amix normalize 不识别** | 删参数 + volume=N |
| 6 | **libass 滤镜中文方块** | drawtext + textfile + STHeiti |
| 7 | **5.0 长 prompt 触发"final generation failed"** | i2i 换装 / 缩短 prompt / 换 4.6 模型 |
| 8 | **`tail -N` 截 image_url** | `tee /tmp/out` 完整保留再 grep |
| 9 | **中文 fontfile 路径含空格** | `${FONT// /\\ }` bash 转义 |
| 10 | **dreamina credit 失败也扣费** | 重试限次（最多 3 次）|

## Credit 与时间预算

**第一集 14 镜头参考成本**（dreamina maestro）：
- 4 张 v2 三视图 ~12 credit
- 14 张分镜单图 ~42 credit
- 14 段 image2video（含失败重试）~350 credit
- **总 ~400 credit**

**总时间 ~5 小时**，其中 **70% 是 image2video 排队等待**（人工时间约 1.5 小时，其余可后台跑）。

## 课程 SOP 对照（哪些可跳过）

| SOP 阶段 | 必做？ | 替代方案 |
|---|---|---|
| 1-3 剧本/分镜/角色介绍 | ✅ 必做 | 用现成或 AI 改写 |
| 4 资产库三视图 | ✅ 必做 | 锚定角色一致性的基础 |
| 5-6 单镜头生图/生视频 | ✅ 必做 | 主体内容 |
| 7 **首尾帧素材** | ⚠️ 可跳 | 用 ffmpeg fade 转场代替 |
| 8 **剪映工程** | ⚠️ 可跳 | 直接 ffmpeg 出 mp4（如要精修再导剪映）|
| 9 成片 | ✅ 必做 | ffmpeg master 拼接 |

## 决策标准

**什么时候用本 Skill 而不是 `jimeng-multishot-replicate`**：
- 中文国内短剧/漫剧（ins 复刻投放走 jimeng）
- 1-3 分钟完整成片含片头片尾（ins 18s 短片走 ffmpeg-reels-pipeline）
- 日漫赛璐璐风格（黑白线稿走 jimeng）

**什么时候用 image2image 而不是 image2video**：
- i2i = 生静态图（每镜头 1 张）
- i2v = 让某张图动起来（5s 视频）
- 顺序：先用 i2i 生 14 张分镜单图，再挑核心几张走 i2v 让它动

## 关键参考

详细模板见 `reference.md`：
- 完整 14 镜头 prompt 集（末世招队员第一集实例）
- 完整 master_v4.sh 拼接脚本（176 行）
- 完整 i2v_polling.py 后台 polling（150 行）
- 完整 watchdog_v4.sh
- 项目目录初始化命令
- BGM 4 段氛围拼接完整 ffmpeg 命令

## 输出格式

调用本 Skill 帮用户做事时：
1. **先确认起点**：用户在哪一阶段？有什么素材？要做完整一集还是只优化某段？
2. **给出当前阶段的具体命令**（dreamina/ffmpeg/python 一键可跑）
3. **预告下一阶段**和耗时/credit 预算
4. **遇到踩坑主动给修复方案**，不让用户撞墙
