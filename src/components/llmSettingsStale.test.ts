import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const read = (rel: string) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");

/**
 * 2026-09-29 用户换了 Azure 的地址和密钥：「无效，得到的还是旧端点数据」。
 *
 * 实际上新端点是通的，毛病在界面三处：
 * 1. 刷新失败时保留旧列表，而**错误只在列表为空时才显示** —— 有旧列表就看不出失败了
 * 2. 改了地址 / 密钥不清旧列表 —— 旧列表是旧端点的
 * 3. 链路换了后端，模型名还是上一个后端的（LM Studio 的 openai/gpt-oss-120b 留在 Azure 上，
 *    后台任务全部失败，下拉框还标着「当前使用」）
 */
describe("改了连接参数，界面不再显示旧端点的东西", () => {
  const settings = read("./LlmSettings.tsx");
  const store = read("../store/chat.ts");

  it("改了地址或密钥：清掉旧列表重拉", () => {
    const saveUrl = settings.slice(settings.indexOf("onSaveUrl={"), settings.indexOf("onSaveKey={"));
    expect(saveUrl).toContain("loadModels(p.id, { reset: true })");
    const saveKey = settings.slice(settings.indexOf("onSaveKey={"), settings.indexOf("onRefresh={"));
    expect(saveKey).toContain("loadModels(p.id, { reset: true })");
    // store 那边：reset 时不拿旧列表兜底
    const load = store.slice(store.indexOf("async loadModels(providerId, opts)"), store.indexOf("async loadAllModels()"));
    expect(load).toContain("opts?.reset ? EMPTY_MODELS");
  });

  it("刷新失败但还有旧列表：错误照样显示", () => {
    expect(settings).toContain('models?.status === "error" && list.length > 0');
    expect(settings).toContain("刷新失败（下面还是上次拉到的");
  });

  it("链路换了后端：模型名清回「跟随后端」", () => {
    const i = settings.indexOf("api.setTierProvider(route.id as Tier, value)");
    const handler = settings.slice(i, i + 400);
    expect(handler).toContain('api.setRouteModel(route.id, "")');
  });
});
