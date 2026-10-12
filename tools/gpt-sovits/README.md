# GPT-SoVITS

零样本音色克隆的 TTS。`upstream/` 是官方仓库的 clone，已 gitignore（连 4.6GB
预训练权重一起），所以安装步骤留在这里。

音色来自[共享音色库](../../voice-library/README.md) —— 和 F5-TTS 用同一套参考。

## 实测数字（M4 Max，MPS）

| | 值 |
|---|---|
| 合成 | 2.7s → 4.2s 音频（**1.5× 实时**） |
| 采样率 | 32kHz（比 Kokoro / F5-TTS 的 24kHz 高） |
| 预训练权重 | 4.6 GB |
| python 依赖 | 约 4 GB |

对比：Kokoro 35× 实时、F5-TTS 2.2×、IndexTTS2 0.25×（不可用）。
它比 F5-TTS 略慢但采样率更高。

## 安装

**不要跑官方的 `install.sh`** —— 它要 conda，还会去装 Xcode Command Line Tools。
用 uv 直接装依赖即可（`deepspeed` / `flash-attn` 是 CUDA 专用的可选项，Mac 上跳过）：

```bash
cd tools/gpt-sovits
git clone --depth 1 https://github.com/RVC-Boss/GPT-SoVITS.git upstream
cd upstream
uv venv --python 3.10 .venv
uv pip install --python .venv/bin/python torch torchaudio
uv pip install --python .venv/bin/python -r requirements.txt
# torchaudio 新版把音频加载挪到了 torchcodec 上，requirements 里没列
uv pip install --python .venv/bin/python torchcodec
```

预训练权重（4.25GB 压缩包）：

```bash
curl -L -o pretrained_models.zip \
  "https://huggingface.co/XXXXRT/GPT-SoVITS-Pretrained/resolve/main/pretrained_models.zip"
unzip -q pretrained_models.zip -d GPT_SoVITS/ && rm pretrained_models.zip
```

改推理配置走 MPS。**`is_half` 必须是 false** —— MPS 上的半精度会因为算子回落
而更慢（IndexTTS2 实测 fp16 比 fp32 慢四倍多）：

```bash
sed -i '' 's/device: cuda/device: mps/; s/is_half: true/is_half: false/' \
  GPT_SoVITS/configs/tts_infer.yaml
```

## 三个和模型无关的坑

装的时候撞到的，都不在官方文档里：

1. **`torchcodec` 缺失** → 报 `TorchCodec is required for load_with_torchcodec`。
   torchaudio 新版把音频加载改到它上面了，而 `requirements.txt` 没跟上。
2. **NLTK 数据包缺失** → 报 `Resource 'averaged_perceptron_tagger_eng' not found`。
   而且**不能在仓库目录里下载** —— nltk 会试着从 cwd import `regex`，
   撞上仓库里的同名文件就失败。换个目录下载。
3. **`PYTORCH_ENABLE_MPS_FALLBACK=1` 是必需的**，它有几个算子在 MPS 上没实现，
   不给这个变量会直接崩在启动阶段。

## 启动

`anima.sh` 会在配置选了 `gpt_sovits` 时自动拉起来。手工起：

```bash
cd upstream
PYTORCH_ENABLE_MPS_FALLBACK=1 .venv/bin/python api_v2.py -a 127.0.0.1 -p 9880
```

加载 bert + hubert + 声码器要一会儿，启动脚本给了 180 秒。

## 联调测试

```bash
cd src-tauri
cargo test --test live live_gpt_sovits -- --ignored --nocapture
```

验的是**共享音色库的解析**：引擎配置里的 `ref_audio_path` 刻意留空，参考应当由
音色库按选中的音色名解析出来。原先必须手工填服务端可读的绝对路径**加**那段话的
文字，还得自己保证两者一致 —— 写错的表现是"音色像但吐字乱"，极难归因。
