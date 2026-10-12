import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { deleteLabel } from "./DeleteAssetButton";

const read = (rel: string) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
const BTN = read("./DeleteAssetButton.tsx");
const CARD = read("./AssetCard.tsx");
const STEP = read("../steps/AssetsStep.tsx");
const BENCH = read("../assets/AssetWorkbench.tsx");

/**
 * 2026-09-30 用户：「你得给我角色删除按钮，不然我没地方删」。资产页点卡片直接开工作台，
 * 删除在另一个详情面板里，而那个面板从 5cbc149 起就不出现了。
 */
describe("资产页能删资产", () => {
  it("两步：先准备删，再点一次才删；被镜头引用拦下之后给强制删", () => {
    expect(deleteLabel("none")).toBe("删除这个资产");
    expect(deleteLabel("arm")).toBe("再点一次删除");
    expect(deleteLabel("force")).toContain("强制删除");
    // 卡片「⋯」菜单里那一行窄，用短的
    expect(deleteLabel("none", true)).toBe("删除资产");
    expect(deleteLabel("force", true)).toBe("强制删除");
    // 第一次点只是准备；被拦下（后端报哪一集哪一镜在用）才进 force
    expect(BTN).toMatch(/if \(step === "none"\) \{\s*setStep\("arm"\);\s*return;/);
    expect(BTN).toContain('await assetsDelete(asset.scope, asset.id, step === "force");');
    expect(BTN).toMatch(/catch \(err\) \{[\s\S]*setStep\("force"\)/);
  });

  /** 用户：「不要把删除按钮直接放在卡片上，在每个卡片右下角增加三个点的菜单按钮，点击弹窗可删除」 */
  it("删除在卡片右下角的「⋯」菜单里（不冒泡成「打开工作台」），资产页的两组卡片和工作台都接上了", () => {
    expect(BTN).toContain("e.stopPropagation();");
    expect(CARD).toContain('<div className="absolute bottom-1.5 right-1.5 z-10">');
    expect(CARD).toContain("<AssetCardMenu asset={asset} onDeleted={onDeleted} />");
    expect(CARD).not.toContain("<DeleteAssetButton");
    // 菜单用应用统一的那套（studio-menu + MenuItem），宽度收在卡片里（卡片最窄 148px）；删除步骤和工作台共用 useAssetDelete
    const MENU = read("./AssetCardMenu.tsx");
    expect(MENU).toContain('className="studio-menu absolute bottom-full right-0 z-30 mb-1 w-[140px]"');
    expect(MENU).toContain("<MenuItem");
    expect(MENU).toContain("useAssetDelete(asset,");
    expect(BTN).toContain("useAssetDelete(asset, onDeleted)");
    expect(STEP.match(/onDeleted=\{\(\) => void reload\(\)\}/g)?.length).toBe(2);
    expect(STEP).toContain("onDeleted={() => {\n            setBench(null);");
    expect(BENCH).toContain("<DeleteAssetButton");
    expect(BENCH).toContain("onDeleted={onDeleted}");
  });
});

/**
 * 2026-09-30 v0.42.120 把卡片包进一层 div 放删除按钮，**卡片大小全乱了**：没图的卡缩成字那么宽，同一排高低不齐。
 * 原来卡片按钮自己是网格格子、被拉满整格；包了之后被拉满的是外层，`<button>` 默认按内容收缩。
 * 修法实测过（同一份编译出来的样式做对照页看过）：外层 `flex`（纵向自动拉齐一排）+ 按钮 `w-full`
 */
describe("卡片包了外层之后大小不能变", () => {
  it("外层是 flex，卡片按钮撑满宽度", () => {
    expect(CARD).toContain('<div className="group relative flex">');
    expect(CARD).toContain('className="studio-tile flex w-full flex-col text-left"');
  });
});

/** 2026-09-30 用户：角色卡上下各一大块黑 —— 图片位写死 3:4，而项目的角色画幅是横版 */
describe("卡片图片位按项目的画幅", () => {
  it("项目里按每一类的出图画幅；不给（资产库页跨项目）才按类型的老规矩", () => {
    expect(CARD).toContain("const ratio = (aspect && ASPECT_CLASS[aspect]) || CARD_ASPECT[asset.kind];");
    expect(CARD).toContain('landscape: "aspect-video"');
    expect(STEP).toContain("const aspectOf = (k: string) => effectiveGen(k, null, project?.model_settings).aspect;");
    expect(STEP.match(/aspectOf=\{aspectOf\}/g)?.length).toBe(2);
  });
});

