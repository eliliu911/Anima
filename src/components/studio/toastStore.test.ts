import { beforeEach, describe, expect, it } from "vitest";
import { DEFAULT_TTL, describeCounts, runTask, useToasts } from "./toastStore";

beforeEach(() => useToasts.setState({ items: [], closed: [] }));

describe("通知条", () => {
  /**
   * 「分析中…」和「分析完成」是**同一件事的两个状态**，不是两条消息。
   * 各推一条的话屏幕上会同时挂着两条 —— 用户截图里 LumenX 就有这个毛病
   * （一条转圈的 + 两条完成的叠在一起）。
   */
  it("进度条被就地替换成结果，不是再多一条", () => {
    const id = useToasts.getState().push({ kind: "progress", title: "分析中…" });
    useToasts.getState().resolve(id, { kind: "success", title: "完成" });
    const items = useToasts.getState().items;
    expect(items).toHaveLength(1);
    expect(items[0].id).toBe(id);
    expect(items[0].kind).toBe("success");
    expect(items[0].title).toBe("完成");
  });

  it("替换时 ttl 跟着新类型走 —— 否则成功那条会永远挂着", () => {
    const id = useToasts.getState().push({ kind: "progress", title: "分析中…" });
    expect(useToasts.getState().items[0].ttl).toBeNull();
    useToasts.getState().resolve(id, { kind: "success", title: "完成" });
    expect(useToasts.getState().items[0].ttl).toBe(DEFAULT_TTL.success);
  });

  /** 进度和报错都要留着：进度没结束不能消失，报错要留给人读完。 */
  it("进度和报错不自动消失", () => {
    expect(DEFAULT_TTL.progress).toBeNull();
    expect(DEFAULT_TTL.error).toBeNull();
    expect(DEFAULT_TTL.success).toBeGreaterThan(0);
  });

  it("多条并存，关掉一条不影响别的", () => {
    const a = useToasts.getState().push({ kind: "error", title: "甲" });
    useToasts.getState().push({ kind: "error", title: "乙" });
    useToasts.getState().dismiss(a);
    expect(useToasts.getState().items.map((x) => x.title)).toEqual(["乙"]);
  });

  it("resolve 一个已经关掉的 id 不会凭空造出一条", () => {
    const id = useToasts.getState().push({ kind: "progress", title: "分析中…" });
    useToasts.getState().dismiss(id);
    useToasts.getState().resolve(id, { kind: "success", title: "完成" });
    expect(useToasts.getState().items).toHaveLength(0);
  });
});

describe("数量描述", () => {
  /**
   * **零的那类不写出来。**「发现 3 个角色、0 个场景、0 个道具」
   * 读起来像出了问题，而那段剧本本来就没有场景。
   */
  it("只列出非零的", () => {
    expect(describeCounts([
      { label: "角色", n: 3 },
      { label: "场景", n: 0 },
      { label: "道具", n: 1 },
    ])).toBe("发现 3 个角色、1 个道具。");
  });

  it("全是零时说清楚，而不是空句子", () => {
    expect(describeCounts([{ label: "角色", n: 0 }])).toBe("什么都没找到");
  });
});

describe("按固定 id 放（后台任务用）", () => {
  it("同一个 id 就地换，不叠第二条", () => {
    const { upsert } = useToasts.getState();
    upsert("job-1", { kind: "progress", title: "在跑" });
    upsert("job-1", { kind: "progress", title: "第 2 场" });
    upsert("job-1", { kind: "success", title: "好了" });
    const items = useToasts.getState().items;
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({ id: "job-1", kind: "success", title: "好了", ttl: DEFAULT_TTL.success });
  });

  it("没有就新加，不同任务各一条", () => {
    const { upsert } = useToasts.getState();
    upsert("job-1", { kind: "progress", title: "甲" });
    upsert("job-2", { kind: "progress", title: "乙" });
    expect(useToasts.getState().items.map((x) => x.id)).toEqual(["job-1", "job-2"]);
  });

  /** 进度每场更新一次，关掉了还弹回来就等于关不掉。 */
  it("手动关掉的进度条不再弹回来，但结果照样显示", () => {
    const s = useToasts.getState();
    s.upsert("job-1", { kind: "progress", title: "第 1 场" });
    s.dismiss("job-1");
    useToasts.getState().upsert("job-1", { kind: "progress", title: "第 2 场" });
    expect(useToasts.getState().items).toHaveLength(0);
    useToasts.getState().upsert("job-1", { kind: "error", title: "失败了" });
    expect(useToasts.getState().items.map((x) => x.title)).toEqual(["失败了"]);
  });
});

describe("runTask：前端短任务的通知", () => {
  it("在跑是一条进度，完成就地换成结果，返回值原样给回", async () => {
    let finish!: (v: number) => void;
    const p = runTask(
      { title: "合成中…", failure: "合成失败", success: (n) => ({ title: `合成好了`, body: `${n} 个镜头` }) },
      () => new Promise<number>((r) => (finish = r)),
    );
    expect(useToasts.getState().items).toMatchObject([{ kind: "progress", title: "合成中…" }]);
    finish(7);
    await expect(p).resolves.toBe(7);
    expect(useToasts.getState().items).toMatchObject([{ kind: "success", title: "合成好了", body: "7 个镜头" }]);
  });

  /** 页面原来的错误处理（onError 横幅）不用改。 */
  it("失败显示原文并把错误抛回去", async () => {
    const p = runTask({ title: "分析中…", failure: "分析失败", success: () => ({ title: "好了" }) }, () =>
      Promise.reject(new Error("LM Studio 没有加载模型")),
    );
    await expect(p).rejects.toThrow("LM Studio 没有加载模型");
    expect(useToasts.getState().items).toMatchObject([{ kind: "error", title: "分析失败", body: "Error: LM Studio 没有加载模型" }]);
  });

  it("用户关掉进度条，结果照样显示", async () => {
    let finish!: () => void;
    const p = runTask({ title: "跑", failure: "败", success: () => ({ title: "好了" }) }, () => new Promise<void>((r) => (finish = r)));
    useToasts.getState().dismiss(useToasts.getState().items[0].id);
    finish();
    await p;
    expect(useToasts.getState().items.map((x) => x.title)).toEqual(["好了"]);
  });

  it("两个任务各自一条，不串", async () => {
    await Promise.all([
      runTask({ title: "甲", failure: "x", success: () => ({ title: "甲好了" }) }, async () => 1),
      runTask({ title: "乙", failure: "x", success: () => ({ title: "乙好了" }) }, async () => 2),
    ]);
    expect(useToasts.getState().items.map((x) => x.title)).toEqual(["甲好了", "乙好了"]);
  });
});
