# 负向词工程

借鉴 BFL 的 `negative-prompt-alternatives.md`（它的核心论点：很多情况下
**用正向措辞替代负面词更有效**）与本机实测。

## 已经写进代码的默认负向词

`image_tool::default_negative_prompt()`，模型不传 `negative_prompt` 时自动用：

```
blurry, low quality, deformed, distorted, text errors, ugly,
bad anatomy, watermark, pixelated,
extra limbs, extra legs, extra arms, extra fingers, extra digits,
fused fingers, missing fingers, malformed hands, mutated hands,
poorly drawn hands, poorly drawn feet, malformed feet,
bad proportions, disfigured, mutation, cropped
```

**为什么要点名到这个程度**：本机对照实测，只有笼统的 `bad anatomy` 时人物悬空、
支撑腿够不到地面；加上具体词之后站姿正常。**SDXL 的负向词是逐词起作用的**，
抽象形容词覆盖不到具体失败模式。

## 按场景追加

| 场景 | 追加 |
|:---|:---|
| 角色设定卡 | `3d render, cgi, plastic, doll, cartoon, anime, toy figure`（**代码里已自动追加**） |
| 写实人像 | `illustration, painting, drawing, sketch, anime` |
| 产品/静物 | `hands, person, human` |
| 干净背景 | `cluttered background, busy background, text, logo` |
| 避免加水印文字 | `signature, artist name, username, text overlay` |

## 什么时候正向比负向管用

BFL 那条经验在 SDXL 上也成立 —— 以下用**正向描述**比堆负向词有效：

| 想避免 | 与其写负向 | 不如正向写 |
|:---|:---|:---|
| 杂乱背景 | `cluttered, messy` | `clean white seamless backdrop` |
| 塑料皮肤 | `plastic skin` | `natural skin texture, visible pores` |
| 假光 | `bad lighting` | 具体光位，见 lighting.md |
| 半身出画 | `cropped` | `full body, head to toe, feet visible` |

## 第三档：正向配额（positive allocation）

上面那张表是「换成正面说法」。还有更狠的一档：**给不想要的东西分配一个位置**。

来源是 Higgsfield 的 DISCIPLINE 框架
（[OSideMedia/higgsfield-ai-prompt-skill](https://github.com/OSideMedia/higgsfield-ai-prompt-skill)）：

> "Say what you want, not what you avoid — the words you write are the words you
> summon, including the ones inside a 'no'."

他们记了一个「颜色战争」的案例：负向清单 `no yellow` **完全失效**，
换成正向陈述也不够，最后奏效的是**把黄色圈进一个地方**：

```
❌ 负向清单     no yellow
⚠️ 正向陈述     主色是冷青绿
✅ 正向配额     主色是冷青绿，黄色只存在于灯泡内部
```

### 本机实测的两例（2026-09-21，《上课不要玩手机》）

| 想要什么 | 光写正向陈述的结果 | 正向配额怎么写 |
|:---|:---|:---|
| 硝烟浓、不像水雾 | 「滚滚白烟」→ 出来像喷雾 | 「浓密**不透明**的灰白色硝烟，像沸腾一样向外膨胀，一两秒内弥漫半个教室，**把光柱整段切断**」 |
| 墙上是红字，不是红底白字 | 「红色横幅」→ 一定是红底白字 | 「「高考必胜」四个大字，**红色只在笔画上**，字的背后就是白墙本身，没有横幅也没有底板」 |

两例的共同点：**给那个属性指定一个归属**（不透明度归给烟、红色归给笔画），
而不是描述它「像什么」或者「不像什么」。

第二例还有一条单独的教训：**物件名自带默认外观**。
「横幅」这个词本身就意味着红底白字 —— 想要别的样子，得把它拆成
「字 + 底」两件东西分别指定，或者干脆不用那个词。

## 负向词治不了的

- **姿势不对** —— 写 `wrong pose` 无效，要 `pose_image`
- **多肢** —— 负向词能降低概率但不可靠（本机实测多肢是**偶发**的，
  4 个种子未必复现）；骨架才是硬约束
- **手指** —— 只能减少，根治要靠分辨率或局部重绘
