import { describe, expect, it, vi } from "vitest";

import { languageNeedsSync, syncLanguageToConfig } from "../index";

/**
 * **设置里显示简体中文，重启后托盘菜单还是英文。**
 *
 * 托盘是 Rust 在启动时建的，只读得到 `config.json`；界面语言却只存在
 * 前端的 localStorage 里。两边本该由 `changeAppLanguage` 一起写，
 * 但它**只在用户主动切换时触发** —— 语言从旧版本沿用下来、一次都没重新选过，
 * `config.language` 就永远是 `null`，托盘每次启动退回英文。
 *
 * 实测本机 `config.json` 正是 `"language": null` 而界面是中文。
 */
describe("启动时的语言对账", () => {
  it("config 里是 null 时要补写 —— 就是本机那个现场", () => {
    expect(languageNeedsSync(null, "zh-CN")).toBe(true);
    expect(languageNeedsSync(undefined, "zh-CN")).toBe(true);
  });

  it("两边一致就不写", () => {
    expect(languageNeedsSync("zh-CN", "zh-CN")).toBe(false);
    expect(languageNeedsSync("en-US", "en-US")).toBe(false);
  });

  it("不一致要以界面为准补写", () => {
    expect(languageNeedsSync("en-US", "zh-CN")).toBe(true);
  });

  it("界面语言还没定下来时不写 —— 别把空值盖进 config", () => {
    expect(languageNeedsSync("zh-CN", "")).toBe(false);
  });

  it("补写会真的调用写入，并把界面语言传下去", async () => {
    const write = vi.fn().mockResolvedValue(undefined);
    const did = await syncLanguageToConfig(async () => null, write, "zh-CN");
    expect(did).toBe(true);
    expect(write).toHaveBeenCalledWith("zh-CN");
  });

  it("一致时一次都不写", async () => {
    const write = vi.fn();
    const did = await syncLanguageToConfig(async () => "zh-CN", write, "zh-CN");
    expect(did).toBe(false);
    expect(write).not.toHaveBeenCalled();
  });

  it("读配置失败不能把界面拖垮", async () => {
    // 顶多托盘这次还是旧语言，不该抛出去
    const did = await syncLanguageToConfig(
      async () => {
        throw new Error("后端还没起来");
      },
      vi.fn(),
      "zh-CN",
    );
    expect(did).toBe(false);
  });
});
