"""
修 PyTorch 在 Apple Silicon（MPS）上的一个算子 bug：
**半精度（bf16/fp16）的 `F.gelu(approximate="tanh")` 输入 ≥ ~30 就吐 NaN。**

## 怎么查到的（2026-09-13）

Qwen-Image-Edit-2511 在这台 M4 Max 上出纯黑图，任务状态 success，不报错。
逐段排除：文本编码输出 NaN=0 → 权重文件 840 个量化缩放值、1093 个浮点权重全部有限 →
去掉 Lightning LoRA / CFGNorm 照样黑 → 不带参考图、512×512 照样黑（不是长序列注意力）→
逐模块挂钩子，第一个出非有限值的地方是 `transformer_blocks.0.img_mlp`：
线性层输出有限，紧接着的 `F.gelu(approximate="tanh")` 之后变 NaN。

最后用 9 个数就复现了，不需要任何模型：

    输入              [1, 10, 30, 40, 45, 100, 1000]
    CPU  任何精度      正确
    MPS  float32       正确
    MPS  bfloat16      [0.84, 10, NaN, NaN, NaN, NaN, NaN]   ← bug
    MPS  bf16, approximate="none"   正确

tanh 近似里有 `x³` 这一项；MPS 的半精度内核内部像是按 fp16 范围算的，
|x| 过了三十几就溢出成 Inf，再经过 tanh/乘法变成 NaN。

**为什么 `--fp32-unet` 救不了**：int8 量化模型的计算精度由 ComfyUI 按量化配置选，
实测是 `manual_cast=bfloat16`，不看 `--fp32-unet`。

## 修法

只在「MPS + 半精度 + approximate="tanh"」这一个组合下，把这一步提到 fp32 算完再转回原精度。
结果在 fp32 里算对，再转回原精度 —— 实测和 CPU fp32 参考的最大偏差 0.0013，
全部来自转回 bf16 的舍入，不是公式的差别（最初这里写的是「逐位一致」，量了一下不是）。
其它任何情况原样调用，不碰。
代价是那一个激活函数多一次 fp32 拷贝。

修完：同一个模型、同样输入，8 步采样结果 NaN 从 65536/65536 变成 0/65536。

## 为什么是 ComfyUI 自定义节点

ComfyUI 启动时 import 所有 custom_nodes，这里在 import 时打补丁，
对所有模型、所有工作流生效，不用改任何节点。源码放在 Anima 仓库里，
ComfyUI 那边是指过来的符号链接 —— 只有一份，改了就是改了。
"""
import torch
import torch.nn.functional as F

TAG = "[anima-mps-gelu-fix]"


def _bug_present() -> bool:
    """这台机器上 bug 在不在。在才打补丁 —— PyTorch 哪天修好了，补丁自动变成空操作。"""
    if not torch.backends.mps.is_available():
        return False
    x = torch.tensor([45.0], device="mps", dtype=torch.bfloat16)
    y = F.gelu(x, approximate="tanh")
    return not bool(torch.isfinite(y).all().item())


def install() -> bool:
    if getattr(F.gelu, "_anima_mps_gelu_fix", False):
        return True
    try:
        present = _bug_present()
    except Exception as e:  # 自检失败不能把 ComfyUI 带崩
        print(f"{TAG} 自检失败，不打补丁：{e}", flush=True)
        return False
    if not present:
        print(f"{TAG} 这台机器上没有这个 bug，不打补丁", flush=True)
        return False

    orig = F.gelu

    def gelu(input, approximate="none"):
        if (approximate == "tanh" and input.device.type == "mps"
                and input.dtype in (torch.bfloat16, torch.float16)):
            return orig(input.float(), approximate="tanh").to(input.dtype)
        return orig(input, approximate=approximate)

    gelu._anima_mps_gelu_fix = True
    F.gelu = gelu
    torch.nn.functional.gelu = gelu
    print(f"{TAG} MPS 半精度 tanh-GELU 会吐 NaN —— 已改为该情况下按 fp32 计算", flush=True)
    return True


def _install_status() -> str:
    """`patched`（这台机器有 bug、补丁已打）/ `not-needed`（没有 bug）/ `failed`（自检失败，没打）。"""
    if getattr(F.gelu, "_anima_mps_gelu_fix", False):
        return "patched"
    try:
        return "failed" if _bug_present() else "not-needed"
    except Exception:
        return "failed"


install()
STATUS = _install_status()


class AnimaMpsGeluFix:
    """
    **它存在的唯一用处是让 Anima 查得到补丁在不在。**

    Anima 出 Qwen-Image-Edit 之前会问 ComfyUI 的 `/object_info` 有没有这个节点。
    能查到，说明**正在跑的这个 ComfyUI 进程**真的加载了补丁 —— 比读启动日志可靠
    （日志随重启改名；文件在盘上不代表进程加载过）。

    **自检失败时不注册这个节点**：那种情况补丁没打上，Qwen 在这台 Mac 上会出纯黑图，
    宁可让 Anima 的检查拦下来报清楚，也不能让它以为有保护。
    """

    @classmethod
    def INPUT_TYPES(cls):
        return {"required": {}}

    RETURN_TYPES = ("STRING",)
    FUNCTION = "status"
    CATEGORY = "anima"
    DESCRIPTION = f"MPS 半精度 tanh-GELU 修复：{STATUS}"

    def status(self):
        return (STATUS,)


NODE_CLASS_MAPPINGS = {} if STATUS == "failed" else {"AnimaMpsGeluFix": AnimaMpsGeluFix}
NODE_DISPLAY_NAME_MAPPINGS = {k: "Anima · MPS GELU 修复状态" for k in NODE_CLASS_MAPPINGS}
