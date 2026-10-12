import { describe, expect, it } from "vitest";
import { keyEditable, keySourceHint, keySourceLabel, type KeySource } from "./keySource";

const ALL: (KeySource | null)[] = ["env", "file", "keychain", null];

describe("key 来源", () => {
  /** **只有环境变量那条改不掉。** 判错的话用户会在设置里白改半天 */
  it("只有环境变量不可改", () => {
    expect(keyEditable("env")).toBe(false);
    expect(keyEditable("file")).toBe(true);
    expect(keyEditable("keychain")).toBe(true);
    expect(keyEditable(null)).toBe(true);
  });

  it("每种来源都有自己的说法，不能撞车", () => {
    const hints = ALL.map(keySourceHint);
    expect(new Set(hints).size).toBe(ALL.length);
    for (const h of hints) expect(h.length).toBeGreaterThan(10);
  });

  /** 没配 key 时不该显示一个来源胶囊 */
  it("没配时标签为空", () => {
    expect(keySourceLabel(null)).toBe("");
    expect(keySourceLabel(undefined)).toBe("");
    for (const s of ["env", "file", "keychain"] as KeySource[]) {
      expect(keySourceLabel(s)).not.toBe("");
    }
  });

  /** 钥匙串那条要告诉用户「再存一次就迁走了」，否则他不知道怎么摆脱弹框 */
  it("钥匙串的说明要给出路", () => {
    expect(keySourceHint("keychain")).toContain("重新保存");
  });
});
