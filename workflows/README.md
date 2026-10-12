# 本机适配版 ComfyUI 工作流（Apple Silicon / MPS）

从 kenpechi 的 LTX 2.3 工作流和 LTX-2.5-Director 官方示例改造而来，
目标是**这台机器**：M4 Max / 137GB 统一内存 / MPS，没有 CUDA。

## 两份文件

| 文件 | 用途 | 状态 |
|---|---|---|
| `LTX25_I2V_Full_Anima_MPS.json` | **图生视频·全功能**（NAG + 音频 + 分块解码 + ESRGAN + RIFE） | **已实跑、看过图、验过音轨** |
| `LTX25_I2V_Anima_MPS.json` | 图生视频·精简版（无 NAG/音频/补帧，跑得快） | **已实跑并看过图** |
| `LTX25_Director_Anima_MPS.json` | 多段时间线编排（主轨/音轨/运动轨，多镜头），已补 NAG + RIFE | 结构已核对，**Director 节点本身未实跑** |

### 全功能版实测

```
512×512 源图 → 2x latent 超分 → 2x RealESRGAN → RIFE 2x
= 2048×2048 / 97 帧 / 48fps / 601 秒 + AAC 48kHz 立体声
```

源图的脸保住了、按提示词转头微笑、逐帧确认在动、画面无伪影、音轨有实际内容且无 NaN。

### 新装的三个节点包

| 包 | 为什么 |
|---|---|
| `kijai/ComfyUI-KJNodes` | **`LTX2_NAG`** 在这里 —— 这是最值钱的一项 |
| `Lightricks/ComfyUI-LTXVideo` | `LTXVTiledVAEDecode` 等官方节点 |
| `Fannovel16/ComfyUI-Frame-Interpolation` | RIFE 补帧。用 `requirements-no-cupy.txt`（cupy 只有 CUDA 版），RIFE 走 ComfyUI 的 `get_torch_device()`，在 MPS 上正常 |

三个包都确认无导入失败。KJNodes 里 sageattention 的 import 在函数体内，不用就不会触发。

## 相对原版改了什么，为什么

| 原版 | 问题 | 改成 |
|---|---|---|
| `*-comfy-int8-convrot.safetensors` | **int8：MPS 缺 `aten::_int_mm`** | bf16 |
| `ltx-2.3-22b-dev-fp8` / `gemma_3_12B_it_fp4_mixed` | **fp8/fp4 是 NVIDIA 专用张量格式** | LTX 2.5 的 bf16 原版 |
| `PathchSageAttentionKJ`、`LTX2MemoryEfficientSageAttentionPatch` | SageAttention 是 CUDA-only | 摘除 |
| `RTXVideoSuperResolution` | NVIDIA RTX 专用 | 换 RealESRGAN_x2plus |
| `VAELoaderKJ` ×N | KJNodes 没装 | 内置 `VAELoader` |
| `VHS_VideoCombine` | VideoHelperSuite 没装 | 内置 `CreateVideo` + `SaveVideo` |
| `LTXVImgToVideoInplaceKJ` | 同上 | 内置 `LTXVImgToVideoInplace` |
| `LTXVTiledVAEDecode` / `LTXVSpatioTemporalTiledVAEDecode` | LTXVideo 包没装 | 内置 `VAEDecodeTiled` |
| `Power Lora Loader (rgthree)`、`easy loraNames` | 包没装 | 内置 `LoraLoaderModelOnly`（或直接摘除） |
| 蒸馏 LoRA | 本机 transformer **本身就是 distilled** | 摘除，避免重复叠加 |
| `vae_ltx\x.safetensors` | Windows 反斜杠子目录 | 去掉前缀 |

**一个第三方节点包都没有新装** —— 缺的全部有 ComfyUI 内置替代。

## 没能补上的

| 功能 | 原因 |
|---|---|
| `LTX2_NAG` 注意力引导 | 要装 Lightricks LTXVideo 包；画质增强，非必需 |
| `RIFEInterpolation` 补帧 | 要装补帧包，未验证 Apple Silicon 兼容性 |
| `@ref` 参考图 / Ghost Mask / MSR | **LTX 2.5 没学过 2.3 训练出来的那个行为**，作者在 2.5 分支里主动关掉了 —— 留着会让输出崩坏。参考图生视频在 Anima 里走 MiniMax H3。 |

## 和原版比，还差什么（已基本补齐）

装了三个节点包之后，原版会影响成片的功能**基本都补上了**：

| 原版有 | 现在 | 说明 |
|---|---|---|
| `LTX2_NAG (11, 0.25, 2.5)` | ✅ **已补** | 蒸馏模型 cfg=1 时 CFG 是关的，**负向提示词本来完全不起作用**；NAG 在注意力层里把负向概念推出去。参数和原版逐字一致 |
| 音频（EmptyLatentAudio + AudioVAEDecode + 音量） | ✅ **已补** | 实测出片带 AAC 48kHz 立体声，RMS 37.5、峰值 353、全部有限值 —— **没有出现 MPS 上的 NaN** |
| `LTXVTiledVAEDecode` 分块解码 | ✅ **已补** | 用回原版那个 KJNodes 节点（4×4 分块），内存吃紧时靠它 |
| RealESRGAN 最终超分 | ✅ **已补** | `RealESRGAN_x2plus.pth` 已下载 |
| RIFE 补帧 | ✅ **已补**（略有差异） | `RIFE VFI` 2x：**24 → 48fps**。原版是 24→60，RIFE VFI 的倍数是整数，2 最接近 |
| `RTXVideoSuperResolution` | ❌ 补不了 | NVIDIA RTX 专用；RealESRGAN 已顶替其作用 |
| `FireRed-Image-Edit-Lightning` LoRA | ❌ 未加 | 它是 **LTX 2.3 训练的**，挂到 2.5 上大概率静默失效。`LTX2LoraLoaderAdvanced` 节点已可用，找到 2.5 版权重再挂 |
| `@ref` / Ghost Mask / MSR 参考功能 | ❌ 补不了 | **2.5 没学过 2.3 训练出来的那个行为**，作者在 2.5 分支里主动关掉 —— 留着会让输出崩坏。参考图生视频在 Anima 里走 MiniMax H3 |

### 一处结构上的不同（等价，但要知道）

原版用 `LTXVImgToVideoInplaceKJ` 直接在**音视频合并后**的 latent 上贴图。
内置的 `LTXVImgToVideoInplace` 处理不了那个 `NestedTensor`
（报 `'NestedTensor' object has no attribute 'clone'`），而 KJ 版的
`num_images` 是动态组合框、API 格式不好写。

所以这里改成**先在纯视频 latent 上贴图、再拼音频**：

```
EmptyLTXVLatentVideo → ImgToVideoInplace(0.75) → ConcatAVLatent → 采样
采样 → SeparateAVLatent → 视频超分 → ImgToVideoInplace(1.0) → 再 ConcatAVLatent → 二阶段
```

顺序不同、结果等价，而且只用内置节点。

### 分辨率是我改小的

| | 原版 | 这里 |
|---|---|---|
| `EmptyLTXVLatentVideo` | 1024×1024 / 97 帧 | **512×512 / 49 帧** |

**这是实跑验证过的那组**（512² → 2x latent → 2x ESRGAN → RIFE 2x = **2048² / 97 帧 / 48fps / 601 秒**）。
原版那组在这台机器上没验过，两级超分后是 4096²，内存很可能不够。
要按原版跑就改回去，先看一眼 `/system_stats` 的空闲内存。

## 用之前要知道的

- **内存**：LTX 在这台机器上是内存瓶颈。跑之前确认 ComfyUI 空闲内存充足
  （`/system_stats`），必要时先关掉 LM Studio。分块解码（`VAEDecodeTiled`）
  已经接上，就是为了缓解这一段。
- **空 audio latent 在 MPS 上可能解出 NaN 并拖垮 SaveVideo**（Anima 代码里
  记着这条）。I2V 这份**默认没接音频**；要音频的话接上
  `LTXVEmptyLatentAudio` + `LTXVConcatAVLatent`，并且先小规模验证。
- **别同时装 LTX Director 的 2.3 包和 2.5 包** —— 老版本注册同名节点，
  ComfyUI 会静默只加载一个。

## 改造脚本

- `scripts/adapt-ltx-director-workflow.py` —— 官方示例更新后重跑即可，不用手改
- `scripts/comfy-api-to-ui.py` —— 把验证过的 API 图转成编辑器能打开的 UI 格式，
  插槽顺序按 `/object_info` 的真实签名生成（顺序错了会连错线**且不报错**）
