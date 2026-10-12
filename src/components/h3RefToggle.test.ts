import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

/** 2026-10-06 用户定：H3 参考生视频的原生 Ref2VA 做成开关、默认兼容模式 */
describe("H3 参考生视频权重开关", () => {
  const src = readFileSync(new URL("./TtsEngineSettings.tsx", import.meta.url), "utf8");
  const api = readFileSync(new URL("../api/index.ts", import.meta.url), "utf8");

  it("读不到配置时当兼容模式", () => {
    expect(src).toContain("st.config?.h3_native_ref2va) ?? false");
  });
  it("没装原生权重：那张卡不能点，高亮停在兼容模式", () => {
    expect(src).toContain("disabled: !h3NativeInstalled");
    expect(src).toContain("(h3Native && h3NativeInstalled) === e.on");
  });
  it("点了写回后端再刷新配置", () => {
    expect(src).toMatch(/await setH3NativeRef2va\(on\);\s*await refreshConfig\(\);/);
    expect(api).toContain('invoke("set_h3_native_ref2va", { enabled })');
  });
});
