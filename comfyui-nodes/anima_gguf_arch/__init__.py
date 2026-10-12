"""
让 ComfyUI-GGUF 认得出 **Qwen-Image 2.1** 的 GGUF。

## 病症（2026-09-22）

`UnetLoaderGGUF` 直接报：

    ValueError: This model is not currently supported - (Unknown model architecture!)

**这不是「Qwen 2.1 在 ComfyUI 上跑不了」，是一块砖的问题** —— ComfyUI 核心认得它：
`comfy/model_detection.py` 里有 `qwen_image21` 分支，拿这份 GGUF 的张量名和形状
喂进 `detect_unet_config` 能正确解出
`{image_model: qwen_image21, in_channels: 64, num_layers: 32, ...}`。

卡住的是 ComfyUI-GGUF 那一层。它的流程是：

1. 读 GGUF 的 `general.architecture` 元数据；
2. **读不到就退回按张量名猜**（`tools/convert.py` 的 `detect_arch`）。

而 `abenzerps/Qwen-Image-2.1-GGUF` 这份文件**一个 KV 元数据都没有**
（`gguf` 读出来只有 `GGUF.version` / `tensor_count` / `kv_count` 三个内置字段），
于是走到第 2 步；`detect_arch` 的 `arch_list` 里有 flux / sd3 / wan / lumina2 等 11 个，
**一个 Qwen 都没有** —— 因为正常的 Qwen GGUF 都带元数据，根本走不到这条退路。

## 修法

在 `arch_list` 里补一条 Qwen-Image 2.1。判据**照抄 ComfyUI 核心自己的那五个键**
（`model_detection.py` 的 `qwen_image21_keys`），不另发明一套 —— 两处分叉的表现是
「GGUF 加载器放行了、核心又认不出」，报的错还是同一句看不懂的话。

`arch` 报成 `"qwen_image"`：那个字符串后面只用来过 `loader.py` 的 `IMG_ARCH_LIST`
白名单，真正决定模型结构的是核心的 `detect_unet_config`。

## 为什么是我们仓库里的一个节点，而不是改 ComfyUI-GGUF

改它的 `tools/convert.py` 是改第三方代码：`git pull` 一次就没了，
**而且没了之后不会有任何提示**，只会在下次出图时冒出同一句错。
放在这里跟 `anima_mps_gelu_fix` 是同一个办法 —— 源码在 Anima 仓库，
ComfyUI 那边是符号链接。

## 第二个入口：带了元数据，但名字不在白名单里（2026-09-26）

`abenzerps/Qwen-Image-2.1-Uncensored-GGUF` 的 UC 版**带了** `general.architecture`，
值是 `qwen_image21`。于是不走上面那条退路，直接撞 `loader.py` 的 `IMG_ARCH_LIST`
白名单（里面只有 `qwen_image`）：

    ValueError: Unexpected architecture type in GGUF file: 'qwen_image21'

对出图模型来说 `arch_str` **只用来过这道白名单**（`gguf_sd_loader` 里再没有别的分支
看它），模型结构照样由核心的 `detect_unet_config` 按张量定。所以往白名单里补一个
`qwen_image21` 就够了，不需要改名或转换文件。

## 哪天可以删

两件都要满足：用的 GGUF 带元数据（第一个入口用不上了），**并且** ComfyUI-GGUF
上游的 `IMG_ARCH_LIST` 自己有了 `qwen_image21`（第二个入口用不上了）。
判据不是「看起来能跑」—— 是把这个链接摘掉、重启 ComfyUI、再出一张图。
"""
import importlib
import logging
import os
import sys
import threading

TAG = "[anima-gguf-arch]"

# 照抄 `comfy/model_detection.py` 的 `qwen_image21_keys`。
QWEN_IMAGE_21_KEYS = (
    "txt_in.text_norm.weight",
    "modulation.1.weight",
    "transformer_blocks.0.attn.norm_q.weight",
    "img_in.weight",
    "proj_out.weight",
)

#: UC 版 GGUF 自己写在 `general.architecture` 里的名字。
QWEN_IMAGE_21_ARCH = "qwen_image21"

#: ComfyUI-GGUF 那个包在 `sys.modules` 里的键长什么样。
#:
#: **不是 `"ComfyUI-GGUF"`。** ComfyUI 的 `load_custom_node` 用的是
#: `module_path.replace(".", "_x_")` —— 也就是那个目录的**绝对路径**。
#: 按包名去 `sys.modules` 里找，找一辈子也找不到（第一版就是这么写的）。
_PKG_DIR_NAME = "ComfyUI-GGUF"


def _find_pkg_name():
    """在 `sys.modules` 里找 ComfyUI-GGUF 那个包的键。没加载返回 `None`。"""
    for name, mod in list(sys.modules.items()):
        if not isinstance(name, str) or not name.endswith(_PKG_DIR_NAME):
            continue
        # 排除 `…/ComfyUI-GGUF.tools` 这类子模块，只要包本身
        if os.path.basename(name.replace("_x_", ".")) == _PKG_DIR_NAME and getattr(mod, "__path__", None):
            return name
    return None


def _patch_once() -> str:
    pkg = _find_pkg_name()
    if pkg is None:
        return "pending"
    try:
        convert = importlib.import_module(f"{pkg}.tools.convert")
    except Exception as e:  # noqa: BLE001 —— 上游改了目录结构就老实报出来
        return f"failed: 导入 {pkg}.tools.convert 失败（{e}）"

    # 第二个入口：UC 版带元数据 `qwen_image21`，要过 loader 的白名单。
    try:
        loader = importlib.import_module(f"{pkg}.loader")
    except Exception as e:  # noqa: BLE001
        return f"failed: 导入 {pkg}.loader 失败（{e}）"
    img_arch = getattr(loader, "IMG_ARCH_LIST", None)
    if not isinstance(img_arch, set):
        return "failed: ComfyUI-GGUF 的 loader 里没有 IMG_ARCH_LIST（或者它不再是 set）"
    # 原地改：`gguf_sd_loader` 每次调用都按名字读这个全局，换成新对象它看不见
    img_arch.add(QWEN_IMAGE_21_ARCH)

    arch_list = getattr(convert, "arch_list", None)
    if arch_list is None:
        return "failed: ComfyUI-GGUF 的 tools.convert 里没有 arch_list"
    if any(getattr(a, "_anima_qwen_image_21", False) for a in arch_list):
        return "ok（已经装过）"

    class ModelQwenImage21(convert.ModelTemplate):
        arch = "qwen_image"
        shape_fix = False
        keys_detect = [list(QWEN_IMAGE_21_KEYS)]
        _anima_qwen_image_21 = True

    # **插在最前面。** 这几个键很具体，不会误吞别的架构；而排在后面的话，
    # 万一将来某条更宽松的规则先匹配上，它就再也轮不到，且没有任何症状。
    arch_list.insert(0, ModelQwenImage21)

    # 自检：真的按这套键认出来了吗。
    try:
        got = convert.detect_arch(set(QWEN_IMAGE_21_KEYS)).arch
    except Exception as e:  # noqa: BLE001
        return f"failed: 装上了但自检没过（{e}）"
    if got != "qwen_image":
        return f"failed: 自检认成了 {got!r}"
    return "ok"


STATUS = _patch_once()


def _retry_in_background():
    """ComfyUI-GGUF 还没加载时，等它。

    **自定义节点的加载顺序是 `os.listdir` 的顺序，没有排序**（`nodes.py`），
    所以「它一定在我们之前」是靠不住的 —— 今天成立，换台机器、加个节点就可能不成立。
    等到它出现为止（上限 120 秒），然后补。
    """
    global STATUS
    for _ in range(240):
        threading.Event().wait(0.5)
        s = _patch_once()
        if s != "pending":
            STATUS = s
            logging.info("%s %s（等到 ComfyUI-GGUF 加载完之后）", TAG, s)
            return
    STATUS = "failed: 等了 120 秒也没见到 ComfyUI-GGUF"
    logging.warning("%s %s", TAG, STATUS)


logging.info("%s %s", TAG, STATUS)
if STATUS == "pending":
    threading.Thread(target=_retry_in_background, daemon=True).start()


class AnimaGgufArch:
    """**唯一用处是让 Anima 查得到补丁在不在。**

    Anima 出 Qwen-Image 2.1 之前问 ComfyUI 的 `/object_info` 有没有这个节点。
    查得到 = **正在跑的这个进程**真的补上了架构表；文件在盘上不代表进程加载过。

    **和 `AnimaMpsGeluFix` 不同，这里只有确凿失败才不注册。**
    那边补丁没打上是**静默**出黑图，所以宁可错杀；这边补丁没打上是
    `UnetLoaderGGUF` 当场抛 `Unknown model architecture!`，任务直接失败、
    错误原文就在结果里 —— 响的失败不需要提前拦，拦了反而会因为
    「还在等 ComfyUI-GGUF 加载」这种时序问题误伤。
    """

    @classmethod
    def INPUT_TYPES(cls):
        return {"required": {}}

    RETURN_TYPES = ("STRING",)
    FUNCTION = "status"
    CATEGORY = "anima"
    DESCRIPTION = f"Qwen-Image 2.1 的 GGUF 架构识别：{STATUS}"

    def status(self):
        return (STATUS,)


NODE_CLASS_MAPPINGS = {} if STATUS.startswith("failed") else {"AnimaGgufArch": AnimaGgufArch}
NODE_DISPLAY_NAME_MAPPINGS = {k: "Anima · GGUF 架构识别状态" for k in NODE_CLASS_MAPPINGS}
