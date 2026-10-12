import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { MAX_BYTES, uploadNote, uploadProblem } from "./uploadImage";

describe("资产上传图片", () => {
  it("PNG / JPG / WebP 能传", () => {
    for (const type of ["image/png", "image/jpeg", "image/webp"]) {
      expect(uploadProblem({ name: "a", type, size: 1000 })).toBeNull();
    }
  });

  /** SVG / HEIC 进出片链路会静默出错，在入口挡住并说清楚。 */
  it("别的格式、太大的挡住并说原因", () => {
    expect(uploadProblem({ name: "logo.svg", type: "image/svg+xml", size: 10 })).toContain("logo.svg");
    expect(uploadProblem({ name: "IMG.heic", type: "image/heic", size: 10 })).toContain("不是 PNG / JPG / WebP");
    expect(uploadProblem({ name: "big.png", type: "image/png", size: MAX_BYTES + 1 })).toContain("超过 30 MB");
    expect(uploadProblem({ name: "ok.png", type: "image/png", size: MAX_BYTES })).toBeNull();
  });

  it("来源记成「上传：文件名」", () => {
    expect(uploadNote("皇帝.png")).toBe("上传：皇帝.png");
  });
});

const strip = (src: string) => src.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
const read = (rel: string) => strip(readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8"));

describe("上传入口接线", () => {
  it("加成资产的一版图（后端会设为封面），传完刷新", () => {
    const btn = read("./UploadImageButton.tsx");
    expect(btn).toContain('media: { kind: "data_url", data_url: dataUrl }');
    expect(btn).toContain("prompt_used: uploadNote(file.name)");
    expect(btn).toContain("if (added > 0) onDone();");
    expect(btn).toContain("const problem = uploadProblem(file);");
  });

  /** 工作台按形象出图：每一处入库都要带形象，漏一处那种图就进了主形象 */
  it("工作台里每一处入库都带形象", () => {
    const src = read("./AssetWorkbench.tsx");
    const calls = src.split("assetsAddVariant({").slice(1).map((c) => c.slice(0, 200));
    expect(calls.length).toBeGreaterThan(0);
    for (const c of calls) expect(c, c).toContain("look_id: lookId");
  });

  it("资产工作台和资产库详情都有入口", () => {
    expect(read("./AssetWorkbench.tsx")).toContain("<UploadImageButton asset={asset} lookId={lookId} onDone={onChanged} />");
    // 资产库详情里按形象传：传到哪个形象就进哪个形象的候选图
    expect(read("../library/AssetInspector.tsx")).toContain("<UploadImageButton asset={asset} lookId={look.id} onDone={onChanged}");
  });
});
