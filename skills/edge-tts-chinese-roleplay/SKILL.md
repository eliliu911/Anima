---
name: edge-tts-chinese-roleplay
description: 帮助用户用免费的微软 edge-tts 神经语音做中文角色化 TTS 配音（多角色声线分配 + SSML rate/pitch 情绪精调 + ffmpeg 多段 mp3 按时间码合到视频音轨），用一套"Yunxi 男主 / Yunjian 反派 / Yunxia 配角 / Xiaoxiao 旁白 4 类声线 + 6 情绪 rate/pitch 表 + adelay+amix 多段合成模板"替代"macOS say 90% 中文 voice 是空音 / 商用 TTS 太贵 / TTS 念稿没情绪"的常见坑。当用户描述"我要给短剧/有声小说/教程/Vlog 配中文 TTS / macOS say 出来都是空音 / 我用 Tingting 配音听起来机器味重 / 我配音念稿一个调没起伏 / 我要多角色对白配音怎么分声线 / 我陆明的爽文台词怎么读出装逼范 / 我系统女音怎么有机械感 / 我反派怎么读出粗暴怒吼 / 我配角小丑怎么读出谄媚感 / 我多段 TTS 怎么按字幕时间码合到视频上 / 我 ffmpeg amix 报错 normalize=0 不识别 / 我 TTS 段音量太小被 BGM 盖掉"时使用；当用户问"什么是 edge-tts / 怎么免费用 Azure 神经语音 / 中文有哪些 voice / YunxiNeural / YunjianNeural / YunxiaNeural / XiaoxiaoNeural 怎么选 / rate +30%/-30% 是什么效果 / pitch +10Hz 怎么用 / SSML 是什么 / 跨段不同声线怎么混合 / ffmpeg adelay amix 多段 mp3 合视频 / 老版 ffmpeg amix normalize 报错 / macOS say 哪些 voice 真能合成中文 / 怎么诊断 voice 是否工作"时使用；当用户在做"AI 短剧配音 / 中文教程 voiceover / 抖音解说 TTS / 有声小说多角色 / 知识付费课程旁白 / Vlog 字幕跟读 / 播客 AI 主持"时使用。提供：1) **macOS say 中文 voice 诊断**（90% 是空音的检测脚本）；2) **edge-tts 安装 + CLI 基本用法**；3) **4 类中文角色声线推荐**（YunxiNeural 阳光男主 / YunjianNeural 粗暴反派 / YunxiaNeural 萌系小丑配角 / XiaoxiaoNeural 温柔旁白系统）；4) **6 类情绪 rate/pitch 速查表**（紧张惊恐 / 阳光调侃 / 沉重 OS / 霸气定海 / 系统机械感 / 谄媚拔尖）；5) **ffmpeg adelay+amix 多段合视频模板**（按字幕时间码插每段 mp3 + volume 补偿 amix 自动衰减）；6) **ffmpeg 老版 normalize= 不识别踩坑**（删参数 + volume=N 手动补）；7) **中文台词节奏调整心法**（叹号 emphasis / 省略号 break / 一字台词降速）；8) **末世招队员第一集 15 段实证 prompt**（角色+rate+pitch+文本完整范例）。**与同源姐妹的边界**：本 Skill 是横向工具，跨任何中文内容场景（短剧/教程/有声小说/Vlog/解说）。被 `ai-mandrama-pipeline` 作为配音阶段子流程调用。不依赖任何视频生成工具，纯文本→mp3→合视频音轨。**不适用于**：英文/日文 TTS（edge-tts 支持但本 skill 模板针对中文优化）/ 真人录音（用录音笔+剪映）/ 需要商用授权的高端配音（用字节火山引擎 TTS、Azure 付费版、阿里云）/ 实时 TTS（edge-tts 是离线批量场景）。
name_zh: "edge-tts 中文角色配音（mandrama）"
marker_zh: "配音"
description_zh: "多角色声线分配、SSML 语速音高情绪、按时间码合进视频。"
source: "https://github.com/cyuanxv/ai-mandrama-skills @ 0142fc1（MIT）。Anima 只在 frontmatter 加了中文名/标记/说明/来源四个字段，正文未改。"
---

# edge-tts 中文角色化 TTS 配音

## 何时使用本 Skill

- 任何中文内容需要"多角色 TTS 配音"且预算为 0（免费）
- 短剧/动态漫/有声小说/教程/Vlog/抖音解说/课程旁白
- 已踩过 macOS `say` 90% 中文 voice 是空音的坑
- 需要给视频按字幕时间码插入多段配音
- 念稿 TTS 没情绪起伏，要 SSML 调 rate/pitch

## 核心方法

### 第一步：装 edge-tts（一次性）

```bash
pip3 install edge-tts
edge-tts --list-voices | grep -E "zh-CN" | head -15
```

预期输出（**实际可用的中文 voice**）：
- `zh-CN-XiaoxiaoNeural` Female / News, Novel / Warm
- `zh-CN-XiaoyiNeural` Female / Cartoon, Novel / Lively
- `zh-CN-YunjianNeural` Male / Sports, Novel / Passion
- `zh-CN-YunxiNeural` Male / Novel / Lively, Sunshine
- `zh-CN-YunxiaNeural` Male / Cartoon, Novel / Cute
- `zh-CN-YunyangNeural` Male / News / Professional, Reliable
- 辽宁-XiaobeiNeural Female / Dialect / Humorous（东北话）
- 陕西-XiaoniNeural Female / Dialect / Bright（陕西话）

### 第二步：角色声线分配（4 类标配）

| 角色类型 | Voice | 风格描述 | 典型应用 |
|---|---|---|---|
| **爽文男主 / 阳光小说男** | `zh-CN-YunxiNeural` | 阳光男主，活泼，小说朗读型 | 陆明 / 韩立 / 男 vlog 主播 |
| **反派 / 粗暴男 / 怒吼路人** | `zh-CN-YunjianNeural` | 体育激情，粗暴男 | 队长1 / 路人惊恐喊救命 / 反派吼叫 |
| **萌系小丑 / 配角谄媚** | `zh-CN-YunxiaNeural` | 卡通可爱搞笑 | 配角男争抢 / 弹幕小丑 / 反差萌 |
| **系统女音 / 温柔旁白** | `zh-CN-XiaoxiaoNeural` | 温柔，新闻级，小说级 | 系统提示 / 旁白 / 智能助手 |

### 第三步：6 类情绪 rate/pitch 速查

```bash
edge-tts --voice "<voice>" --rate "<rate>" --pitch "<pitch>" \
  --text "<台词>" --write-media output.mp3
```

| 情绪 | rate | pitch | 适用 | 示例 |
|---|---|---|---|---|
| **紧张惊恐喊叫** | +25% | +10Hz | 路人惊恐 / 反派怒吼 | "救命啊！" |
| **阳光调侃** | +8% | +3Hz | 男主轻松摆烂 | "不准训练哈！" |
| **沉重 OS 自嘲** | -5% | -3Hz | 男主内心独白 | "我叫陆明，上辈子内卷加班猝死" |
| **霸气定海一字** | -30% | -5Hz | 男主装逼定场 | "起——！" |
| **系统机械感** | -15% | -2Hz | 系统女音降速 | "恭喜宿主激活系统" |
| **谄媚小丑拔尖** | +20% | +10Hz | 配角谄媚抢戏 | "大佬选我！" |

### 第四步：ffmpeg 合到视频音轨

**核心命令**（多段 mp3 按时间码 adelay + volume 补 + amix 混合）：

```bash
ffmpeg -y -i input_video.mp4 \
  -i seg_01.mp3 -i seg_02.mp3 ... -i seg_15.mp3 \
  -i bgm.wav \
  -filter_complex "
[1:a]aresample=44100,adelay=500|500,volume=14[t1];
[2:a]aresample=44100,adelay=5500|5500,volume=14[t2];
...
[15:a]aresample=44100,adelay=52500|52500,volume=14[t15];
[16:a]aresample=44100,adelay=4000|4000,volume=0.18[bgm];
[t1][t2]...[t15][bgm]amix=inputs=16:dropout_transition=0[aout]" \
  -map 0:v -map "[aout]" \
  -c:v copy -c:a aac -b:a 192k -shortest \
  output.mp4
```

**关键参数说明**：
- `adelay=Nms|Nms` — 把这段音频在合成轨上向后挪 N 毫秒（左右声道都挪）
- `volume=14` — 每段 TTS 提升 14 倍（补 amix 自动衰减，N 段输入 amix 默认按 1/N 衰减）
- `volume=0.18` — BGM 降到 18% 不盖配音
- `amix=inputs=N:dropout_transition=0` — N 段混合，**不要加 `normalize=0`（老版 ffmpeg 不识别）**
- `-c:v copy` — 视频流不重编码
- `-shortest` — 最后一段音频结束时截断输出

### 第五步：节奏调整心法

文本本身的标点也参与节奏控制（edge-tts 在标点处自然停顿）：
- **加叹号 → 加强语气** → "起！" vs "起。"
- **加省略号 → 制造悬念断点** → "现在……你们，还觉得我是骗子吗？"
- **加破折号 → 拉长元音** → "起——！"
- **加逗号 → 自然停顿** → "抱歉各位，我已经选好队员了，就是她。"

## 关键案例

### 案例 1：末世招队员第一集 15 段配音（完整）

```bash
# 旁白沉重
edge-tts --voice "zh-CN-XiaoxiaoNeural" --rate="-10%" --pitch="-5Hz" \
  --text "末世，爆发第三年。" --write-media seg_01a.mp3

# 路人惊恐
edge-tts --voice "zh-CN-YunjianNeural" --rate="+25%" --pitch="+10Hz" \
  --text "救——命！啊！！" --write-media seg_01b.mp3

# 男主摆烂调侃
edge-tts --voice "zh-CN-YunxiNeural" --rate="+8%" --pitch="+3Hz" \
  --text "诶诶，不准训练哈！在我这，你只管摸鱼躺平就行。" --write-media seg_02.mp3

# 男主 OS 自嘲
edge-tts --voice "zh-CN-YunxiNeural" --rate="-5%" --pitch="-3Hz" \
  --text "我叫陆明。上辈子，内卷加班，猝死。" --write-media seg_04.mp3

# 系统机械感
edge-tts --voice "zh-CN-XiaoxiaoNeural" --rate="-15%" --pitch="-2Hz" \
  --text "恭喜宿主，激活末世摆烂系统。" --write-media seg_05.mp3

# 男主决绝
edge-tts --voice "zh-CN-YunxiNeural" --rate="-5%" --pitch="+2Hz" \
  --text "这一世，我陆明，要靠摆烂活到最后！" --write-media seg_06.mp3

# 反派怒吼
edge-tts --voice "zh-CN-YunjianNeural" --rate="+20%" --pitch="+5Hz" \
  --text "滚开！一个F级废物！还想进我的队！做梦！" --write-media seg_07.mp3

# 男主激动夸张
edge-tts --voice "zh-CN-YunxiNeural" --rate="+15%" --pitch="+8Hz" \
  --text "太好了！我的小队，就缺你这种人才！" --write-media seg_09.mp3

# 男主霸气一字
edge-tts --voice "zh-CN-YunxiNeural" --rate="-30%" --pitch="-5Hz" \
  --text "起——！" --write-media seg_10.mp3

# 男主戏谑
edge-tts --voice "zh-CN-YunxiNeural" --rate="-5%" --pitch="+0Hz" \
  --text "现在……你们，还觉得我是骗子吗？" --write-media seg_11.mp3

# 配角谄媚
edge-tts --voice "zh-CN-YunxiaNeural" --rate="+20%" --pitch="+10Hz" \
  --text "大佬选我！我有罐头！" --write-media seg_12.mp3

# 系统结算机械
edge-tts --voice "zh-CN-XiaoxiaoNeural" --rate="-15%" --pitch="-3Hz" \
  --text "F级治愈异能，废物级。补助奖励，已发放。" --write-media seg_14.mp3
```

## 决策标准

### 什么时候选哪个 voice

- **男主是"爽文/痞帅/温柔小说男主"** → `YunxiNeural`
- **男主是"老成稳重/新闻播音腔"** → `YunyangNeural`
- **男主是"萌系/可爱/反差搞笑"** → `YunxiaNeural`
- **场景需要"愤怒/激情/呐喊/恐惧"** → `YunjianNeural`
- **女声"温柔/旁白/系统"** → `XiaoxiaoNeural`
- **女声"清纯少女/活泼/卡通"** → `XiaoyiNeural`
- **方言搞笑（东北/陕西）** → `liaoning-XiaobeiNeural` / `shaanxi-XiaoniNeural`

### 什么时候不用 edge-tts

- 商业付费短剧/电视剧 → 真人配音演员
- 需要中英混读自然衔接 → Azure 付费版 / 火山引擎
- 实时对话（如 voicebot） → edge-tts 是批量场景，延迟不行

## 常见陷阱

### 陷阱 1：macOS `say` 90% 中文 voice 是空音

```bash
say -v "?" | grep zh_CN  # 列出 10 个看似可用的中文 voice
say -v Reed -o /tmp/test.aiff "你好"  # 但其实大部分不工作
afinfo /tmp/test.aiff | grep duration  # 显示 "0.015964 sec" = 空音
```

**实际能合成中文的只有**：`Tingting` (zh_CN 女) + `Sinji` (zh_HK 男/女混港味)。其他 Reed/Eddy/Sandy/Rocko/Flo/Grandma/Grandpa/Shelley 都是"虚假可用"——`say -v "?"` 列得出来但合不出真实音频，输出 0.02s 的头部空文件。

**解决**：跳过 macOS say，直接用 edge-tts。

### 陷阱 2：ffmpeg 老版 amix `normalize=` 不识别

```bash
# ❌ 报错 "Option 'normalize' not found"
amix=inputs=15:normalize=0[out]

# ✅ 删参数 + 用 volume= 手动补回 amix 自动衰减
amix=inputs=15[mix]; [mix]volume=14[out]
```

**原因**：`normalize` 是较新 ffmpeg 版本的 amix 滤镜选项，2018 年的 N-92585 版本没有。

### 陷阱 3：TTS 段音量被 BGM 盖掉

amix 默认每段输入按 1/N 自动衰减。15 段 TTS + 1 BGM 混合 → TTS 音量降到 1/16。**修复**：每段 TTS 用 `volume=14`（≈ N，补回去）+ BGM 用 `volume=0.18`（降到 18%）。

### 陷阱 4：多段 mp3 合到视频后音轨与画面错位

**原因**：mp3 编码有 ~50ms 静音 padding。**修复**：在 adelay 上做 -50ms 微调，或者用 wav 替代 mp3 输入（edge-tts 输出 mp3 但可用 ffmpeg 转 wav 一次性消除 padding）。

## 输出格式

调用本 Skill 帮用户做事时：
1. **先确认场景**：什么内容？多少段对白？什么角色？
2. **角色声线分配建议表**（列哪段用哪个 voice + rate + pitch）
3. **可执行的 edge-tts 命令清单**（每段一行）
4. **ffmpeg 合视频音轨命令**（含时间码 adelay 偏移 + volume 配方）
5. **预告"输出 N 个 mp3 文件 + 最终 mp4"**

## 关键参考

末世招队员第一集 15 段实战参数表（完整 voice + rate + pitch + 文本）见 `reference.md`（如有需要可补充）。
