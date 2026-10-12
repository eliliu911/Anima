import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type { VideoGenNodeData } from "../types";
import { isLongTake, openLongTake, planLongTake, windowLines, SPOKEN_CPS } from "./longTakeWindows";

const node = (windows: string[], secs = 3, engine = "h3"): VideoGenNodeData =>
  ({ engine, durationSecs: secs, longTakeWindows: windows }) as VideoGenNodeData;

describe("一个节点一条长镜头", () => {
  it("判据是窗口表，不是连线", () => {
    expect(isLongTake(node(["a", "b"]))).toBe(true);
    expect(isLongTake(node(["a"]))).toBe(false);
    expect(isLongTake({ engine: "h3", durationSecs: 5 } as VideoGenNodeData)).toBe(false);
    expect(isLongTake(undefined)).toBe(false);
  });

  // 没设清晰度 / 加速档 = 480P、参考的默认 Larry 6 步：实测每秒 6.6 分钟（以前固定 4.5，见下面「预计耗时」）
  it("4×15 秒 = 57.6 秒、约 6.6 小时", () => {
    const p = planLongTake(node(["a", "b", "c", "d"], 15));
    if (!p.ok) throw new Error(p.why);
    expect(p.windows).toBe(4);
    expect(p.netSecs).toBeCloseTo(57.583, 2);
    expect(p.etaHours).toBeCloseTo(6.6, 1);
    expect(p.warning).toBeUndefined();
  });

  it("3+3+3 能跑，但要报实验性和三成重叠", () => {
    const p = planLongTake(node(["a", "b", "c"], 3));
    if (!p.ok) throw new Error(p.why);
    expect(p.warning).toContain("17n+5");
  });

  /**
   * **净时长的两个实测锚点。**
   *
   * 这两个数字来自真跑出来的文件，不是算出来的：
   * 3 个 3 秒窗口 → 175 帧 → 7.292 秒（09-20 的 3×3×3 冒烟测试）；
   * 4 个 4 秒窗口 → 362 帧 → 15.083 秒（09-21 的 4×4）。
   *
   * 直觉公式 `n×窗口秒数 − (n−1)×重叠` 对这两条分别给 7.17 和 **13.25** ——
   * 后者差了一秒八，足以让人以为窗口配少了去加窗口，而他本来就到 15 秒了。
   * 差的那一截是 H3 的 `17n+5` 帧栅格吸附。
   */
  it("净时长对得上两次实测：3×3s=7.292s、4×4s=15.083s", () => {
    const a = planLongTake(node(["a", "b", "c"], 3));
    const b = planLongTake(node(["a", "b", "c", "d"], 4));
    if (!a.ok || !b.ok) throw new Error("应该能跑");
    expect(a.netSecs).toBeCloseTo(175 / 24, 3);
    expect(b.netSecs).toBeCloseTo(362 / 24, 3);
    // 直觉公式给的是这两个，都不对 —— 钉住「不是它」
    expect(b.netSecs).not.toBeCloseTo(4 * 4 - 3 * (22 / 24), 2);
  });

  it("窗口不足、引擎不是 H3、窗口超出 2.5~15 秒，都拦得住并说清怎么改", () => {
    const one = planLongTake(node(["只有一个"]));
    expect(one.ok).toBe(false);
    if (!one.ok) expect(one.why).toContain("加一个窗口");

    const ltx = planLongTake(node(["a", "b"], 5, "ltx"));
    expect(ltx.ok).toBe(false);
    if (!ltx.ok) expect(ltx.why).toContain("H3");

    const tooLong = planLongTake(node(["a", "b"], 20));
    expect(tooLong.ok).toBe(false);
    if (!tooLong.ok) expect(tooLong.why).toContain("窗口数加多");

    const tooShort = planLongTake(node(["a", "b"], 1));
    expect(tooShort.ok).toBe(false);
    if (!tooShort.ok) expect(tooShort.why).toContain("调长");
  });

  /** 点「改成长镜头」不该把已经写好的提示词扔掉 —— 扔了就没人敢点。 */
  it("开长镜头时保留原来的提示词当窗口 1", () => {
    expect(openLongTake("她扣下扳机")).toEqual(["她扣下扳机", ""]);
  });
});

// ── 接线 ──────────────────────────────────────────────────────────
//
// 上面那些是纯函数测试 —— 它们全绿也证明不了面板真的调了这几个函数。
// 09-20 / 09-21 的变异测试连着抓到过三次「算得对但没接上」。
describe("面板接线", () => {
  // 出片本身在执行器里（点「生成」），行为测试见 engine/dagExecutor.test.ts「长镜头节点点生成」
  const panel = readFileSync(
    new URL("../nodes/panels/VideoOperationPanel.tsx", import.meta.url),
    "utf8",
  );

  it("面板按节点自己的数据判长镜头，不再顺着连线找链", () => {
    expect(panel).toContain("planLongTake(nodeData)");
    expect(panel).toContain("isLongTake(nodeData)");
    expect(panel).not.toContain("planChain(");
    expect(panel).not.toContain("isChainHead(");
  });


  /** Codex 的验收：窗口序列、上下文重叠、净时长、台词落点，四项都要摊开。 */
  it("常显重叠占比、净时长和每窗台词落点", () => {
    expect(panel).toContain("chain.overlapShare");
    expect(panel).toContain("发布时裁掉");
    expect(panel).toContain("windowLines(prompt, chain.windowSecs)");
    expect(panel).toContain("说不完");
  });

  /**
   * **窗口提示词框必须走 `ComposingTextarea`。**
   * 裸 `<textarea>` 打中文拼音会变成 `qqiqinqinaqin'ai` —— 2026-09-20
   * 在「试音色」节点上撞过一次，同一个坑不能在这里再撞一次。
   */
  it("窗口提示词用 ComposingTextarea，不是裸 textarea", () => {
    expect(panel).toContain("<ComposingTextarea");
    expect(panel).not.toMatch(/<textarea[\s>]/);
  });
});

describe("窗口里的台词落点", () => {
  it("扒出开口时刻和说完要多久", () => {
    const p = 'At 00:0.5, 她转身。<d>[Chinese]你到底想干什么</d> At 00:4.0, 他后退。<d>我没想干什么</d>';
    const ls = windowLines(p, 8);
    expect(ls).toHaveLength(2);
    expect(ls[0].at).toBe(0.5);
    // 「你到底想干什么」七个字 ÷ 4.1
    expect(ls[0].needs).toBeCloseTo(7 / SPOKEN_CPS, 3);
    expect(ls[0].fits).toBe(true);
  });

  /** 2026-10-06 起按 H3 官方规范写镜内时刻（`At MM:SS` 只标切镜）；对齐指令里的 -second mark 不算 */
  it("认官方写法的开口时刻", () => {
    const p =
      "How the reference pictures align with the target video — Picture 1 (from Shot 1) aligns with the 0.00-second mark; Picture 2 (from Shot 1) aligns with the 8.00-second mark.\n\n" +
      "integrated_multimodal_description: [Shot 1] 她转身。Around the 2.5-second mark, she says, <d>[Chinese]你到底想干什么</d> [Shot 2] At 00:05.000, the camera cuts to him. <d>我没想干什么</d>";
    const ls = windowLines(p, 8);
    expect(ls.map((l) => l.at)).toEqual([2.5, 5]);
  });

  /**
   * **长镜头一次提交、不能只重来某一窗口**（H3 潜变量存不了盘）。
   * 「第三个窗口的台词说不完」跑完才发现的代价是整条重来。
   */
  it("说不完要看得出来", () => {
    const p = 'At 00:2.5, 他开口。<d>这句话很长很长很长很长很长很长很长很长很长很长</d>';
    expect(windowLines(p, 3)[0].fits).toBe(false);
    expect(windowLines(p, 15)[0].fits).toBe(true);
  });

  it("标点不占时间，语言标签不算字", () => {
    const a = windowLines('At 00:0, <d>好的</d>', 5)[0];
    const b = windowLines('At 00:0, <d>[Chinese]好的，。！</d>', 5)[0];
    expect(a.needs).toBeCloseTo(b.needs, 6);
  });

  it("没写时间码的台词不瞎猜落点", () => {
    expect(windowLines("<d>没有时间码</d>", 5)).toEqual([]);
  });

  it("没有台词就是空 —— 不制造噪音", () => {
    expect(windowLines("一段普通的画面描写", 5)).toEqual([]);
  });
});

/**
 * **长镜头不选中也要看得出来，而且不能长成好几个出片节点。**
 *
 * 用户 2026-09-21 两次指出：「是一个指令给后端，后端最终也是生成 1 个视频」。
 * 第一次我只在标题栏写了「窗口 N/M」，四个假播放器一个没少，
 * 他第二次的原话是「也没给我改好啊」。
 */
describe("节点标题", () => {
  const src = readFileSync(new URL("../nodes/VideoGenNode.tsx", import.meta.url), "utf8");

  it("标题按节点自己的数据算，不再顺着连线找链", () => {
    expect(src).toContain("planLongTake(data)");
    expect(src).not.toContain("planChain(");
  });

  /** 标题写净时长 —— 写「窗口数 × 秒数」会让人把重叠也算进成片。 */
  it("标题写的是净成片时长", () => {
    expect(src).toContain("约 ${p.netSecs.toFixed(1)}s");
  });

  it("不是长镜头时标题照旧 —— 普通视频节点不受影响", () => {
    expect(src).toContain("longTakeLabel ?? (");
  });
});

/**
 * **一条长镜头就是一个节点。** 预设里再摆成一条链，等于把刚拆掉的
 * 那个误导又原样装回去。
 */
describe("预设", () => {
  const presets = readFileSync(new URL("./workflowPresets.ts", import.meta.url), "utf8");

  it("长镜头预设只摆一个节点、零连线", () => {
    expect(presets).toContain('longTakeWindows: ["", "", ""]');
    expect(presets).toContain("return { nodes: [n], edges: [] };");
  });
});

describe("长镜头预计耗时（查实测表，不再固定每秒 4.5 分钟）", () => {
  // 10-12 星澜 V2：720P + Larry 6 步显示「2.3 小时」，实际约 8 小时
  it("720P + Larry 6 步：参考出片每秒 16.4 分钟", () => {
    const p = planLongTake({ engine: "h3", durationSecs: 15, longTakeWindows: ["a", "b"], width: 1312, height: 736, h3Quality: "staged" } as never);
    expect(p.ok && Math.round(p.etaHours * 10) / 10).toBe(8.2);
  });
  it("480P + LightX2V：每秒 4.6 分钟", () => {
    const p = planLongTake({ engine: "h3", durationSecs: 15, longTakeWindows: ["a", "b"], width: 960, height: 544, h3Quality: "lightx2v" } as never);
    expect(p.ok && Math.round(p.etaHours * 10) / 10).toBe(2.3);
  });
});
