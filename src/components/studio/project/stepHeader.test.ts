import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const SRC = readFileSync(
  fileURLToPath(new URL("./ProjectPage.tsx", import.meta.url)),
  "utf8",
);

describe("步骤页头", () => {
  /**
   * **主操作在配置入口的下面一行，不是并排。**
   *
   * 并排的话「生成分镜」看起来只是第三个设置项 —— 而它是这一页
   * 唯一那个「让事情发生」的按钮，另外两个只是打开对话框。
   * 分行之后它和左边标题块的底边对齐，落在 banner 右下角。
   */
  it("挂载点排在提示词/生成设置那一行之后", () => {
    const row = SRC.indexOf("生成设置");
    const slot = SRC.indexOf("ref={actionRef}");
    expect(row, "配置入口不在了").toBeGreaterThan(0);
    expect(slot, "主操作的挂载点不在了").toBeGreaterThan(0);
    expect(slot, "主操作跑到配置入口前面去了 —— 那就是并排").toBeGreaterThan(row);
  });

  it("右侧是竖排且撑满页头高度（否则贴不到底边）", () => {
    const at = SRC.indexOf("flex-col items-end");
    expect(at, "右侧不是竖排").toBeGreaterThan(0);
    const cls = SRC.slice(at - 60, at + 80);
    expect(cls).toContain("justify-between");
    expect(cls).toContain("self-stretch");
  });

  /**
   * 挂载点必须**始终渲染**：用 state 回调 ref 才能在它出现时触发重渲染。
   * 写成 `useRef` 的话 Provider 会一直拿到初始的 null，
   * 按钮永远不出现，而且不报错。
   */
  it("挂载点用 state 而不是 useRef", () => {
    expect(SRC).toContain("useState<HTMLDivElement | null>(null)");
    expect(SRC).toContain("actionRef={setActionEl}");
  });

  /** 空的时候不能占位，否则上面那行的位置会随步骤切换上下跳。 */
  it("挂载点为空时不占高度", () => {
    const at = SRC.indexOf("ref={actionRef}");
    expect(SRC.slice(at - 120, at + 120)).toContain("empty:hidden");
  });
});

describe("挂载点的健壮性", () => {
  const SRC_ACTION = readFileSync(
    fileURLToPath(new URL("./StepAction.tsx", import.meta.url)),
    "utf8",
  );

  /**
   * **往一个脱离文档的节点 portal，不报错、也看不见。**
   *
   * 挂载点是存在 state 里的 DOM 节点。页头重建时（开发期 Fast Refresh
   * 最典型：state 保留、DOM 重建）state 里留着旧节点，内容就渲染到了
   * 一个看不见的地方 —— 表现是「按钮突然没了」，而代码 / CSS / 布局
   * 全是对的。查这种问题会花掉很久。
   */
  it("挂载点脱离文档时不渲染", () => {
    expect(SRC_ACTION).toContain("isConnected");
  });

  it("拿不到挂载点时渲染 null，不就地渲染", () => {
    expect(SRC_ACTION).toContain("return null");
    // 就地渲染的话按钮会先在内容区闪一下再跳到页头，比不显示更糟
    expect(SRC_ACTION).not.toContain("return <>{children}</>");
  });
});

describe("侧栏的资产计数", () => {
  /**
   * **两级都要数。**
   *
   * 提取出来的实体默认落在分集级（界面上「本集待归属」）。只数项目级的话，
   * 刚提取完六个角色，侧栏第 3 步还是个空圈 —— 而用户明明看见六张卡片。
   * 这是 Phase 13 把提取改成写分集级之后漏掉的一处。
   */
  it("项目级和分集级两处都拉", () => {
    // **锚点必须是取数那一句，不是变量声明。**
    // `indexOf("setAssetTally")` 命中的是上面的 `useState`，
    // 往后取一段就整个错位 —— 这条测试第一版就是这么写挂的。
    const at = SRC.indexOf('assetsList({ kind: "project"');
    expect(at, "没有拉项目级资产").toBeGreaterThan(0);
    const block = SRC.slice(at, at + 400);
    expect(block, "没拉分集级资产").toContain('kind: "episode"');
    expect(block).toContain("setAssetTally");
  });

  /** 换一集之后分集级资产整个换了一批，不重算的话显示的是上一集的数字。 */
  it("换集会重算", () => {
    const at = SRC.indexOf('assetsList({ kind: "project"');
    // externalRev：Chat 改了这个项目时也重算；sel：系列项目在左栏换页时也重算
    expect(SRC.slice(at, at + 1100)).toContain("[projectId, episodeId, step, sel, externalRev]");
  });

  /** 「有图」才算数 —— 没有图的资产锁不住身份，见 stepStates 的注释。 */
  it("区分有图和没图", () => {
    expect(SRC).toContain("withImage");
    expect(SRC).toContain("a.cover_url");
  });
});
