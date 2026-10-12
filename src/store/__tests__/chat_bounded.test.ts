import { describe, it, expect } from "vitest";
import {
  buildBoundedMessages,
  getModelBudgetTier,
  MAX_HISTORY_CHARS,
  modelSupportsVision,
  renderTurnContent,
  stripInlineDataUrls,
  takeUncompacted,
} from "../chat";
import type { ChatTurn } from "../chat";
import type { ConfigView } from "../../api";

describe("chat bounded context & tier tests", () => {
  it("determines model budget tier correctly from provider model_tier", () => {
    const config: ConfigView = {
      providers: [
        {
          id: "p-claude",
          label: "Claude",
          kind: "anthropic",
          base_url: "https://api.anthropic.com",
          model: "claude-3-5",
          supports_tools: true,
          requires_key: true,
          has_key: true,
          key_from_env: false,
          model_tier: "cloud",
        },
        {
          id: "p-lmstudio",
          label: "LM Studio",
          kind: "open_ai_compatible",
          base_url: "http://127.0.0.1:1234/v1",
          model: "qwen-9b",
          supports_tools: true,
          requires_key: false,
          has_key: false,
          key_from_env: false,
          model_tier: "local",
        },
        {
          id: "p-deepseek",
          label: "DeepSeek",
          kind: "open_ai_compatible",
          base_url: "https://api.deepseek.com/v1",
          model: "deepseek-chat",
          supports_tools: true,
          requires_key: true,
          has_key: true,
          key_from_env: false,
          model_tier: "cloud",
        },
      ],
      realtime_provider: "p-lmstudio",
      background_provider: "p-claude",
      tool_fallback_provider: null,
      realtime_model: null,
      background_model: null,
      tool_fallback_model: null,
      disable_thinking: false,
      thinking: { realtime: "off", background: "auto" },
    };

    expect(getModelBudgetTier(config, "realtime")).toBe("local");
    expect(getModelBudgetTier(config, "background")).toBe("cloud");
    expect(MAX_HISTORY_CHARS.local).toBe(48000);
    expect(MAX_HISTORY_CHARS.cloud).toBe(120000);
  });

  it("最近的工具轮次按协议原样发出，且每个 tool_call 都配得上一条 tool 结果", () => {
    const systemPrompt = "System Prompt";
    const turns: ChatTurn[] = [
      {
        id: "turn-1",
        role: "user",
        content: "list files",
        reasoning: "",
        toolCalls: [],
        streaming: false,
      },
      {
        id: "turn-2",
        role: "assistant",
        content: "I checked files",
        reasoning: "",
        toolCalls: [
          {
            id: "call-1",
            name: "view_file",
            arguments: '{"path":"main.rs"}',
          },
        ],
        streaming: false,
      },
      {
        id: "turn-3",
        role: "user",
        content: "now show details",
        reasoning: "",
        toolCalls: [],
        streaming: false,
      },
    ];

    const { messages } = buildBoundedMessages(systemPrompt, turns, "turn-current", 50000);
    // system + user + assistant(tool_calls) + tool 结果 + user
    expect(messages[0].role).toBe("system");
    expect(messages[1].role).toBe("user");
    expect(messages[2].role).toBe("assistant");
    expect(messages[2].content).toContain("I checked files");

    // **最近一轮走协议，不再渲染成 `〔已调用工具: …〕` 文本。**
    //
    // 这条断言原来是反过来的（要求 tool_calls 必须缺席）。改掉的原因见
    // `expandToolTurn` 的注释：那段文本长在 assistant 自己的消息里，是一份
    // 「我平时这么说话」的示范，模型会照抄 —— 抄到连截断标记
    // `…(共 N 字符)` 都写进正文，然后声称图已生成，而 ComfyUI 一次没跑。
    expect(messages[2].tool_calls?.map((c) => c.id)).toEqual(["call-1"]);
    expect(messages[2].content).not.toContain("〔已调用工具");

    // 400 的真正成因是**配不齐**，不是 tool_calls 本身：带 tool_calls 的
    // assistant 消息，每个 id 都必须跟一条 tool 结果。这里没有返回，
    // 也要补占位，不能少发。
    const answered = messages.filter((m) => m.role === "tool").map((m) => m.tool_call_id);
    expect(answered).toEqual(["call-1"]);
    expect(messages[messages.length - 1].role).toBe("user");
  });

  it("strictly enforces budget when tool calls have massive arguments", () => {
    const systemPrompt = "System Prompt";
    const bigContent = "A".repeat(20000);
    const turns: ChatTurn[] = [];

    for (let i = 1; i <= 5; i++) {
      turns.push({
        id: `turn-u-${i}`,
        role: "user",
        content: `write file ${i}`,
        reasoning: "",
        toolCalls: [],
        streaming: false,
      });
      turns.push({
        id: `turn-a-${i}`,
        role: "assistant",
        content: `done ${i}`,
        reasoning: "",
        toolCalls: [
          {
            id: `call-${i}`,
            name: "fs_write_text",
            arguments: JSON.stringify({ path: `file${i}.txt`, content: bigContent }),
          },
        ],
        streaming: false,
      });
    }

    // Add current last turn
    turns.push({
      id: "turn-u-final",
      role: "user",
      content: "final question",
      reasoning: "",
      toolCalls: [],
      streaming: false,
    });

    const budget = 500;
    const { messages } = buildBoundedMessages(systemPrompt, turns, "turn-current", budget);
    const totalChars = messages.reduce((sum, m) => sum + m.content.length, 0);

    // Total length must be strictly within budget (plus possible omission note)
    expect(totalChars).toBeLessThanOrEqual(budget + 100);
    // Should have truncated earlier rounds and included omission note if budget exceeded
    const hasOmission = messages.some((m) => m.content.includes("折叠"));
    expect(hasOmission).toBe(true);
  });
});

describe("工具结果与调用的配对", () => {
  const turn = (overrides: Partial<ChatTurn>): ChatTurn => ({
    id: "t1",
    role: "assistant",
    content: "",
    reasoning: "",
    toolCalls: [],
    streaming: false,
    ...overrides,
  });

  it("同一轮调用两次同名工具时，各自匹配到自己的结果", () => {
    // 按 name 匹配会让两次 view_file 都拿到第一个结果 ——
    // 模型会看到「view_file(b.rs) 返回了 a.rs 的内容」，比没有结果更糟。
    const out = renderTurnContent(
      turn({
        toolCalls: [
          { id: "call_1", name: "view_file", arguments: '{"path":"a.rs"}' },
          { id: "call_2", name: "view_file", arguments: '{"path":"b.rs"}' },
        ],
        toolResults: [
          { callId: "call_1", name: "view_file", content: "AAA_内容" },
          { callId: "call_2", name: "view_file", content: "BBB_内容" },
        ],
      }),
      { compactionTier: "recent" },
      );
    expect(out).toContain("AAA_内容");
    expect(out).toContain("BBB_内容");
    // 顺序也要对：a.rs 那行必须挂 AAA，不能挂 BBB
    const lines = out.split("\n");
    const idxA = lines.findIndex((l) => l.includes("a.rs"));
    expect(lines.slice(idxA, idxA + 2).join("\n")).toContain("AAA_内容");
  });

  it("单次调用时允许按名字兜底（历史数据可能没有 callId）", () => {
    const out = renderTurnContent(
      turn({
        toolCalls: [{ id: "call_x", name: "view_file", arguments: "{}" }],
        toolResults: [{ callId: "", name: "view_file", content: "遗留结果" }],
      }),
      { compactionTier: "recent" },
      );
    expect(out).toContain("遗留结果");
  });

  it("多次调用且 callId 对不上时，宁可不显示也不能配错", () => {
    const out = renderTurnContent(
      turn({
        toolCalls: [
          { id: "call_1", name: "view_file", arguments: '{"path":"a.rs"}' },
          { id: "call_2", name: "view_file", arguments: '{"path":"b.rs"}' },
        ],
        toolResults: [{ callId: "对不上的id", name: "view_file", content: "危险内容" }],
      }),
      { compactionTier: "recent" },
      );
    expect(out).not.toContain("危险内容");
  });

  it("较旧的工具轮压得比最近的狠，避免长期历史撑爆上下文", () => {
    // 断言写成「旧的 < 最近的」而不是钉某个字符数：预算现在是**按轮**分配的
    // （该轮总额在各条结果之间平分），钉死单条数值的话，改调用次数或改总额
    // 都会让这条测试失效，而它想守的性质其实没变。
    const content = "x".repeat(30_000) + "TAIL";
    const mk = (recent: boolean) =>
      renderTurnContent(
        turn({
          toolCalls: [{ id: "call_x", name: "view_file", arguments: "{}" }],
          toolResults: [{ callId: "call_x", name: "view_file", content }],
        }),
        { compactionTier: recent ? "recent" : "medium" },
      );
    const old = mk(false);
    const recent = mk(true);
    expect(old.length).toBeLessThan(recent.length);
    // 两者都必须真的截断，且都告诉模型被截了
    expect(old).not.toContain("TAIL");
    expect(recent).not.toContain("TAIL");
    expect(recent).toContain("共 30004 字符");
    expect(old).toContain("已读取内容: 共 1 行，30004 字符");
  });

  it("最近工具结果保留更大窗口，支持用户说继续时不用立刻重读", () => {
    const content = "x".repeat(1_500) + "RECENT_END";
    const out = renderTurnContent(
      turn({
        toolCalls: [{ id: "call_x", name: "view_file", arguments: "{}" }],
        toolResults: [{ callId: "call_x", name: "view_file", content }],
      }),
      { compactionTier: "recent" },
    );
    expect(out).toContain("RECENT_END");
  });

  it("工具结果里的媒体 data URL 不能进入模型上下文", () => {
    const content = JSON.stringify({
      success: true,
      file_name: "bgm.flac",
      audio_url: `data:audio/flac;base64,${"A".repeat(80_000)}`,
      local_path: "/tmp/bgm.flac",
    });
    const out = renderTurnContent(
      turn({
        toolCalls: [{ id: "call_x", name: "comfyui_generate_song", arguments: "{}" }],
        toolResults: [{ callId: "call_x", name: "comfyui_generate_song", content }],
      }),
      { compactionTier: "recent" },
    );
    expect(out).not.toContain("A".repeat(200));
    expect(out).toContain("data:audio/flac;base64,[omitted");
    expect(out.length).toBeLessThan(2_000);
  });

  it("直接脱敏媒体 data URL，防止入库后撑爆上下文", () => {
    const cleaned = stripInlineDataUrls(`before data:audio/wav;base64,${"B".repeat(10_000)} after`);
    expect(cleaned).not.toContain("B".repeat(200));
    expect(cleaned).toContain("data:audio/wav;base64,[omitted");
    expect(cleaned).toContain("after");
  });

  it("支持三级渐进式衰减 (recent / medium / archived)", () => {
    const t = turn({
      toolCalls: [
        { id: "c1", name: "view_file", arguments: '{"path":"src/main.rs"}' },
        { id: "c2", name: "shell_command", arguments: '{"command":"cargo test"}' },
      ],
      toolResults: [
        { callId: "c1", name: "view_file", content: "fn main() {\n  println!(\"hello\");\n}" },
        { callId: "c2", name: "shell_command", content: "running 10 tests\ntest result: ok. 10 passed; 0 failed" },
      ],
    });

    const recent = renderTurnContent(t, { compactionTier: "recent" });
    const medium = renderTurnContent(t, { compactionTier: "medium" });
    const archived = renderTurnContent(t, { compactionTier: "archived" });

    // Recent: 包含完整代码与测试输出
    expect(recent).toContain("fn main()");
    expect(recent).toContain("running 10 tests");

    // Medium: 包含智能提取的单行指纹状态
    expect(medium).toContain("状态: [已读取内容: 共 3 行，34 字符]");
    expect(medium).toContain("状态: [命令执行成功: running 10 tests]");
    expect(medium).not.toContain("println!(\"hello\")");

    // Archived: 完全剥离输出，仅保留调用签名
    expect(archived).toContain("〔已调用工具: view_file 参数: {\"path\":\"src/main.rs\"}〕");
    expect(archived).toContain("〔已调用工具: shell_command 参数: {\"command\":\"cargo test\"}〕");
    expect(archived).not.toContain("状态:");
    expect(archived).not.toContain("返回:");
  });

  it("正确识别成功但无输出的命令（如 exit status: 0），不误报为命令失败", () => {
    const t = turn({
      toolCalls: [
        { id: "c1", name: "shell_command", arguments: '{"command":"npx tsc --noEmit"}' },
      ],
      toolResults: [
        {
          callId: "c1",
          name: "shell_command",
          content: "命令已结束，退出码：exit status: 0",
          ok: true,
        },
      ],
    });

    const medium = renderTurnContent(t, { compactionTier: "medium" });
    expect(medium).toContain("状态: [命令执行成功: 命令已结束，退出码：exit status: 0]");
    expect(medium).not.toContain("[命令失败");
  });

  it("依据 ok=false 准确识别失败的补丁与命令，避免字符串猜测假阳性", () => {
    const t = turn({
      toolCalls: [
        { id: "c1", name: "apply_patch", arguments: '{"path":"src/app.ts"}' },
        { id: "c2", name: "shell_command", arguments: '{"command":"cargo check"}' },
      ],
      toolResults: [
        {
          callId: "c1",
          name: "apply_patch",
          content: "补丁应用失败：找不到上下文代码",
          ok: false,
        },
        {
          callId: "c2",
          name: "shell_command",
          content: "error[E0425]: cannot find value `foo` in this scope",
          ok: false,
        },
      ],
    });

    const medium = renderTurnContent(t, { compactionTier: "medium" });
    expect(medium).toContain("状态: [文件修改失败: 补丁应用失败：找不到上下文代码]");
    expect(medium).toContain("状态: [命令失败: error[E0425]: cannot find value `foo` in this scope]");
  });

  it("摘要压缩送出的消息使用 medium 轻量档位（避免膨胀摘要请求）", () => {
    const t = turn({
      toolCalls: [
        { id: "c1", name: "view_file", arguments: '{"path":"test.txt"}' },
      ],
      toolResults: [
        {
          callId: "c1",
          name: "view_file",
          content: "LINE 1\nLINE 2\nLINE 3\n" + "x".repeat(10000),
          ok: true,
        },
      ],
    });

    const out = renderTurnContent(t, { compactionTier: "medium" });
    expect(out).toContain("状态: [已读取内容: 共 4 行，10021 字符]");
    expect(out.length).toBeLessThan(500);
  });
});

describe("摘要压缩的增量游标", () => {
  const t = (id: string): ChatTurn => ({
    id,
    role: "user",
    content: id,
    reasoning: "",
    toolCalls: [],
    streaming: false,
  });

  it("第一次压缩：全部送去压", () => {
    expect(takeUncompacted([t("a"), t("b")], null).map((x) => x.id)).toEqual(["a", "b"]);
  });

  it("第二次只送新裁掉的部分，不重复投喂已压过的", () => {
    // compress() 是累积语义：新摘要 = 旧摘要 + 这次裁掉的。
    // 重复投喂 a、b 会让它们在 300 字上限里反复占位，把新内容挤出去。
    const dropped = [t("a"), t("b"), t("c"), t("d")];
    expect(takeUncompacted(dropped, "b").map((x) => x.id)).toEqual(["c", "d"]);
  });

  it("没有新增时返回空，调用方据此跳过压缩", () => {
    expect(takeUncompacted([t("a"), t("b")], "b")).toEqual([]);
  });

  it("游标失效（会话清空过）时兜底全量重压，而不是静默丢弃", () => {
    const dropped = [t("x"), t("y")];
    expect(takeUncompacted(dropped, "不存在的id").map((v) => v.id)).toEqual(["x", "y"]);
  });

  it("历史轮次的图片在滑动窗口中自动脱落 Base64，仅保留当前最新轮次的图片", () => {
    const turns: ChatTurn[] = [
      {
        id: "turn-1",
        role: "user",
        content: "这是上一轮的截图",
        images: ["data:image/png;base64,OLD_IMAGE_DATA"],
        reasoning: "",
        toolCalls: [],
        streaming: false,
      },
      {
        id: "turn-2",
        role: "assistant",
        content: "我分析了上一轮截图",
        reasoning: "",
        toolCalls: [],
        streaming: false,
      },
      {
        id: "turn-3",
        role: "user",
        content: "这是最新提问附带的图片",
        images: ["data:image/png;base64,NEW_IMAGE_DATA"],
        reasoning: "",
        toolCalls: [],
        streaming: false,
      },
    ];

    const result = buildBoundedMessages("Sys", turns, "turn-4");
    // messages: [system, turn-1, turn-2, turn-3]
    expect(result.messages.length).toBe(4);

    // 历史第一轮中的图片已被剔除（images 为 undefined），内容追加轻量标记
    expect(result.messages[1].images).toBeUndefined();
    expect(result.messages[1].content).toContain("[附图：1 张图片已在先前轮次处理]");

    // 最新一轮中的图片被完整保留
    expect(result.messages[3].images).toEqual(["data:image/png;base64,NEW_IMAGE_DATA"]);
  });
});

describe("历史预算与后端上下文的关系", () => {
  // 本机实测的换算比：中文 1.92、Rust 源码 2.82、混合历史 2.30，平均约 2.35
  const CHARS_PER_TOKEN = 2.35;
  // LM Studio 默认按这个 token 数加载模型（模型原生支持 262144，是 LM Studio 收着）
  const LM_STUDIO_DEFAULT_CONTEXT = 32768;

  it("本地预算必须留在 LM Studio 默认加载的上下文之内", () => {
    // 超出的话请求会被后端截断，而且不会有明显报错 —— 表现为"模型莫名其妙忘事"。
    // 真要调过这条线，必须同时抬高 LM Studio 的加载上下文。
    const budgetTokens = MAX_HISTORY_CHARS.local / CHARS_PER_TOKEN;
    expect(budgetTokens).toBeLessThan(LM_STUDIO_DEFAULT_CONTEXT);
  });

  it("本地预算要给回复和工具定义留出余量，不能顶满", () => {
    const budgetTokens = MAX_HISTORY_CHARS.local / CHARS_PER_TOKEN;
    expect(budgetTokens / LM_STUDIO_DEFAULT_CONTEXT).toBeLessThan(0.75);
  });
});

describe("工具结果预算：按轮分配、保护用户意图", () => {
  const user = (id: string, c: string): ChatTurn => ({
    id, role: "user", content: c, reasoning: "", toolCalls: [], streaming: false,
  });
  const toolTurn = (id: string, n: number, content = "我看一下。"): ChatTurn => ({
    id, role: "assistant", content, reasoning: "", streaming: false,
    toolCalls: Array.from({ length: n }, (_, i) => ({
      id: `${id}c${i}`, name: "view_file", arguments: `{"path":"f${i}.ts"}`,
    })),
    toolResults: Array.from({ length: n }, (_, i) => ({
      callId: `${id}c${i}`, name: "view_file", content: "x".repeat(8000),
    })),
  });

  it("一个 13 次调用的工具轮不能吃掉整个预算", () => {
    // 早先每条 8000、没有每轮上限，实测这个场景下单轮占 48020/48000。
    // 13 次调用是那个 LRU 任务的**正常**路径，不是极端情况。
    const one = renderTurnContent(toolTurn("a1", 13), { compactionTier: "recent" });
    expect(one.length).toBeLessThan(MAX_HISTORY_CHARS.local * 0.45);
  });

  it("调用次数翻倍，那一轮占的份额不跟着翻倍", () => {
    const a = renderTurnContent(toolTurn("a", 5), { compactionTier: "recent" });
    const b = renderTurnContent(toolTurn("b", 20), { compactionTier: "recent" });
    // 允许有差异（参数、工具名那些固定开销会随次数涨），但不能是线性关系
    expect(b.length).toBeLessThan(a.length * 2);
  });

  it("用户的原始需求不能被工具结果挤掉", () => {
    // 这是最关键的一条：用户说「继续」时，模型必须还知道要它做什么。
    const turns: ChatTurn[] = [
      user("u0", "第一个问题：项目用什么技术栈？"),
      toolTurn("a0", 2, "技术栈是 Vite + React。"),
      user("u1", "好，帮我实现一个 LRU 缓存，要有测试"),
      toolTurn("a1", 13),
      user("u2", "继续"),
    ];
    const { messages } = buildBoundedMessages("系统提示", turns, "none", MAX_HISTORY_CHARS.local);
    const all = messages.map((m) => m.content).join("\n");
    expect(all).toContain("LRU");
    expect(all).toContain("要有测试");
  });

  it("发出去的总量不超预算（折叠标记也要算进去）", () => {
    // 早先折叠标记是切片之后追加的，没算进预算，实测超 54 字符
    const turns: ChatTurn[] = [
      user("u0", "问题一"),
      toolTurn("a0", 30),
      user("u1", "问题二"),
      toolTurn("a1", 30),
      user("u2", "继续"),
    ];
    const { messages } = buildBoundedMessages("系统提示", turns, "none", MAX_HISTORY_CHARS.local);
    const total = messages.reduce((s, m) => s + m.content.length, 0);
    expect(total).toBeLessThanOrEqual(MAX_HISTORY_CHARS.local);
  });

  it("用户消息本身就超预算时退回先来先得，不能一条都装不下", () => {
    // 保护规则不能反过来把自己饿死
    const huge = user("u0", "y".repeat(MAX_HISTORY_CHARS.local));
    const turns: ChatTurn[] = [huge, toolTurn("a0", 2), user("u1", "继续")];
    const { messages } = buildBoundedMessages("s", turns, "none", MAX_HISTORY_CHARS.local);
    expect(messages.length).toBeGreaterThan(1);
  });
});

describe("视觉能力判定", () => {
  const cfg = (opts: {
    realtimeModel?: string | null;
    providerModel?: string;
  }) =>
    ({
      providers: [
        {
          id: "lmstudio", label: "LM Studio", kind: "open_ai_compatible",
          base_url: "http://127.0.0.1:1234", model: opts.providerModel ?? "qwen/qwen3.8-27b",
          supports_tools: true, requires_key: false, has_key: false,
          key_from_env: false, model_tier: "local",
        },
      ],
      realtime_provider: "lmstudio", background_provider: "lmstudio",
      tool_fallback_provider: null,
      realtime_model: opts.realtimeModel ?? null,
      background_model: null, tool_fallback_model: null,
      disable_thinking: false,
      thinking: { realtime: "off", background: "auto" },
    }) as never;

  const models = (visionModels?: string[]) => ({
    lmstudio: { status: "ok" as const, list: [], visionModels },
  });

  it("模型在视觉名单里 → true", () => {
    expect(
      modelSupportsVision(cfg({ realtimeModel: "qwen/qwen3.8-27b" }), models(["qwen/qwen3.8-27b"]), "realtime"),
    ).toBe(true);
  });

  it("同一个基座的不同打包：uncensored 版没有 mmproj → false", () => {
    // 实测：qwen/qwen3.8-27b 是 vlm、qwen3.8-27b-uncensored 是 llm，
    // 差别只在打包时有没有带 mmproj。**所以不能靠模型名推断。**
    expect(
      modelSupportsVision(
        cfg({ realtimeModel: "qwen3.8-27b-uncensored" }),
        models(["qwen/qwen3.8-27b"]),
        "realtime",
      ),
    ).toBe(false);
  });

  it("查不到能力时是 undefined，不能是 false", () => {
    // 「不知道」和「不支持」是两件事。返回 false 会让界面对着云端后端
    // 谎报"不支持视觉"，而云端多数模型是支持的。
    const r = modelSupportsVision(cfg({}), models(undefined), "realtime");
    expect(r).toBeUndefined();
    expect(r).not.toBe(false);
  });

  it("名单为空数组 = 查到了但确实没有视觉模型 → false", () => {
    // 空数组和 undefined 语义不同：前者是「查过，没有」
    expect(modelSupportsVision(cfg({}), models([]), "realtime")).toBe(false);
  });

  it("没指定链路模型时退回 provider 的默认模型", () => {
    expect(
      modelSupportsVision(
        cfg({ realtimeModel: null, providerModel: "qwen/qwen3.8-27b" }),
        models(["qwen/qwen3.8-27b"]),
        "realtime",
      ),
    ).toBe(true);
  });

  it("按链路取：后台档用 background_model", () => {
    const c = cfg({ realtimeModel: "qwen3.8-27b-uncensored" });
    (c as never as { background_model: string }).background_model = "qwen/qwen3.8-27b";
    expect(modelSupportsVision(c, models(["qwen/qwen3.8-27b"]), "realtime")).toBe(false);
    expect(modelSupportsVision(c, models(["qwen/qwen3.8-27b"]), "background")).toBe(true);
  });
});

/**
 * 文生图那两个 bug 的回归。两个都不报错，只是「说成功了但什么都没发生」。
 *
 * 现场证据：ComfyUI 的 /history 在那段时间只多了两条记录，而用户看到的
 * 「已生成成功」有好几次；模型正文里出现了 `…(共 830 字符)` —— 那个串只可能
 * 由本文件同目录的 `truncateArgs` 生成，真实工具返回里不存在。
 */
describe("工具历史不能变成可模仿的话术", () => {
  const imgTurn = (id: string, callId: string, seed: number): ChatTurn => ({
    id,
    role: "assistant",
    content: "图已经生成好了",
    reasoning: "",
    toolCalls: [
      { id: callId, name: "generate_image", arguments: JSON.stringify({ prompt: "x".repeat(700) }) },
    ],
    toolResults: [
      {
        callId,
        name: "generate_image",
        content: `- **Seed**: \`${seed}\`\n![生成的图像](file:///tmp/a.png)`,
        ok: true,
      },
    ],
    streaming: false,
  });

  it("最近的工具轮次里不出现渲染文本，也不出现截断标记", () => {
    const turns: ChatTurn[] = [
      { id: "u1", role: "user", content: "画一张", reasoning: "", toolCalls: [], streaming: false },
      imgTurn("a1", "call-img-1", 424242),
      { id: "u2", role: "user", content: "裙子改白色", reasoning: "", toolCalls: [], streaming: false },
    ];
    const { messages } = buildBoundedMessages("Sys", turns, "cur", 50000);
    const wire = JSON.stringify(messages);
    expect(wire).not.toContain("〔已调用工具");
    expect(wire).not.toContain("(共 ");
  });
  // 另一半 —— 上一张图的 seed 要活着走到下一轮 —— 在 Rust 侧测：
  // 保护它的是工具返回里 Seed 的**位置**，不是这里的装配逻辑。
  // 见 src-tauri/src/agent_runtime/image_tool.rs 的 `image_summary`。
});
