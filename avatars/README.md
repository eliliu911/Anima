# 面容模型放这里

> 找资源时对着 [`docs/AVATAR_SUPPORT.md`](../docs/AVATAR_SUPPORT.md) 看 ——
> 那份是完整的格式与命名清单。这份只讲怎么放。

把 `.glb` / `.gltf` / `.vrm` 丢进这个目录，设置面板的「面容」页点「重新扫描」
就能选到。**模型文件本身不进版本库**（几十 MB，且是个人素材）——
只有这份说明会被提交。

## 放多个头

直接放多个文件就行。也可以**按角色分子目录**（往下认两层），
界面会按目录分组显示：

    avatars/
    ├── anima.glb              ← 顶层的直接列出来
    ├── alice/
    │   ├── head.gltf
    │   ├── head.bin           ← .gltf 引用的数据，会一起读
    │   └── textures/          ← 贴图目录，不会被当成模型扫描
    │       └── skin.png
    └── bob/
        └── bob.vrm

`.glb` 和 `.vrm` 是自包含的，一个文件就够。**`.gltf` 不是** —— 它是 JSON，
几何数据在 `.bin` 里、贴图在旁边，导出时那几个文件要一起拷进来，
少了会加载出一个没贴图甚至没几何体的模型。

## 先说一句：不放模型也能用

设置面板里有个**内置头**，是代码生成的（`src/avatar/defaultHead.ts`），
不用下载任何东西，15 个口型齐全。它同时是一份参照 —— 想知道 `U` 和 `O`
在嘴形上到底差在哪，看那个文件里的形状表比翻模型快。

先用它确认嘴型链路是通的，再去折腾自己的形象。

## Ready Player Me 已经没有了

之前这份文档写的是去 Ready Player Me 生成模型。**那条路已经不存在**：
Netflix 收购之后，RPM 的 Avatar Creator 和公开 API 于 **2026 年 1 月 31 日
下线**，技术并入 Netflix 自己的产品，没有公开继任者。已有的 avatar 也无法
再生成或更新。

（这份文档最初的版本就是这么写错的 —— 指向了一个已经关停的服务。）

## 现在能用的来源

按「拿到手就能用」排序：

| 来源 | 风格 | 导出 | 口型 | 花费 |
|---|---|---|---|---|
| [Avatar SDK / MetaPerson](https://avatarsdk.com/avatar-creator/) | 写实（照片生成） | **GLB** | blendshape | 有免费在线创建器 |
| [VRoid Studio](https://vroid.com/studio) | 二次元 | **VRM** | 五元音 | 免费 |
| Character Creator 4 / iClone | 写实 | FBX / glTF | viseme 最全 | 付费 |
| MetaHuman | 影视级写实 | 需转换管线 | ARKit 51 | 免费但重 |

**要写实人脸就走 Avatar SDK** —— 它是这几个里唯一「照片进、带 blendshape 的
GLB 出」且不需要装引擎的。

MetaHuman 质量最高，但它是 Unreal 生态：拿到 web 能用的 GLB 需要走
[插件或转换管线](https://github.com/smorchj/metahuman-to-glb)（社区那条能导出
51 个 ARKit blendshape）。要装 Unreal，为一个头像来说成本不低。

**ComfyUI 那类资源库不行。** 那里面是扩散模型（checkpoint / LoRA），产出图像和
视频。就算装了 3D 节点（TripoSR / InstantMesh / Hunyuan3D），出来的也是**静态
网格** —— 没有骨骼、没有 blendshape，嘴是实心的，没有任何可驱动的通道。
从那里到「能对口型的头」中间还隔着绑定和混合形状制作，比找模型本身难得多。

`.vrm` 就是 `.glb` 多了个扩展段，直接丢进来即可，不用转格式。

## 自动识别

加载模型时会遍历所有 mesh 的 `morphTargetDictionary`，按精度从高到低试三套
命名（实现与测试在 `src/avatar/morphMap.ts`）：

| 优先级 | 命名 | 例子 | 精度 |
|---|---|---|---|
| 1 | Oculus viseme | `viseme_aa`、`aa`、`V_aa` | 一一对应，无损失 |
| 2 | ARKit blendshape | `jawOpen`、`mouthPucker` | 拼近似，区分度有限 |
| 3 | 五元音 | `Fcl_MTH_A`、`aa/ih/ou/ee/oh`、`あいうえお` | 辅音靠元音近似 |

设置面板会显示**认出了几个通道**、以及**用的是哪一套**。

## 嘴不动的时候看这里

只有两种原因，面板上能直接分辨：

- **认出 0 个通道，且模型没有任何 morph target** —— 导出时没勾口型/表情，
  重新导出。
- **认出 0 个通道，但模型有一堆 morph target** —— 命名不认识。面板会把
  实际的名字列出来，把它们加进 `morphMap.ts` 的映射表即可。

两种情况模型都能正常加载、正常显示，Three.js 一个错都不报 —— 所以别去怀疑
音频或时间戳那一段。想单独确认前面几环，用「面容」页里的**口型监视器**：
它直接画出 15 个通道的实时权重，不依赖任何模型。
