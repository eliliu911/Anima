# 光影词库（SDXL 有效项）

借鉴 OpenMontage 的 prompting 目录与 Seedance 的 cinematography.md，
只保留在本机 RealVisXL 上确实有响应的词。

## 光位

| 中文 | 提示词 | 效果 |
|:---|:---|:---|
| 顺光 | `front lighting, flat lighting` | 平、少阴影，适合设定卡 |
| 侧光 | `side lighting, split lighting` | 立体感强，塑造轮廓 |
| 逆光/轮廓光 | `backlit, rim light, contre-jour` | 发丝边缘发光，最出氛围 |
| 顶光 | `top lighting, overhead light` | 眼窝阴影重，压抑感 |
| 伦勃朗光 | `Rembrandt lighting` | 颊侧三角光，经典人像 |
| 蝴蝶光 | `butterfly lighting, paramount lighting` | 鼻下蝶形影，时尚感 |

## 光质

`soft diffused light`（柔）· `hard direct light`（硬）· `dappled light`（斑驳）
`volumetric light, god rays`（体积光）· `practical lights`（画面内光源）

## 色温与时段

`golden hour`（黄昏暖光，最讨好肤色）· `blue hour`（蓝调时刻）
`overcast, soft grey light`（阴天，均匀无影）· `neon lit, cyberpunk lighting`
`candlelit, warm firelight` · `moonlight, cool blue tone`

## 摄影棚

`studio lighting, softbox` · `three-point lighting` · `high key`（明亮低反差）
`low key, chiaroscuro`（暗调高反差）· `white seamless backdrop`（设定卡用）

## 注意

- **`cinematic lighting` 这类笼统词效果很弱** —— 和「画质块要具体」是同一条。
  写具体光位比写「电影感」有用得多。
- 逆光 + 浅景深容易让脸欠曝，人像加一句 `fill light on face`。
