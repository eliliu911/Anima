# 镜头与景别词库

## 景别（SDXL 响应中等，配合 width/height 更稳）

| 景别 | 提示词 | 建议画幅 |
|:---|:---|:---|
| 大远景 | `extreme long shot, establishing shot` | 横 1216×832 |
| 全景 | `full body shot, head to toe` | **竖 832×1216** |
| 中景 | `medium shot, waist up` | 竖或方 |
| 近景 | `close-up, head and shoulders` | 方 1024×1024 |
| 特写 | `extreme close-up` | 方 |

**全身人物一定用竖画幅。** 1:1 下人物被压缩，脸和手的像素预算都不够 ——
这也是手指畸形最常见的间接原因。

## 焦段（对透视和背景虚化有实际影响）

`24mm wide angle`（广角，透视夸张，环境交代）
`35mm`（纪实感）· `50mm`（接近人眼）
`85mm portrait lens`（人像标准，背景压缩，脸型讨好）
`135mm telephoto, compressed background`（强压缩）

配 `shallow depth of field, bokeh` 或 `deep focus, everything in focus`。

## 机位与角度

`eye level` · `low angle`（仰拍，显高显强势）· `high angle`（俯拍，显弱）
`dutch angle`（倾斜，不安）· `over the shoulder`（对话）· `bird's eye view`

## 运镜（**只对视频链路有意义**）

图片链路写运镜词是浪费 token —— 静止图没有运动。视频提示词里才写：

`camera tracking alongside` · `slow push in` / `dolly in` · `pull out`
`pan left / right` · `tilt up / down` · `handheld, subtle shake`
`static camera, locked off`（固定机位）· `orbit around subject`

**借自 LTX skill 的一条规则：写了 `static camera` 就不要再出现任何运动动词** ——
两者矛盾时模型要么忽略其一，要么产生画面自相矛盾的抖动。
