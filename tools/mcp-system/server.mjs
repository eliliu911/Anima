#!/usr/bin/env node
/**
 * Anima 的系统控制 MCP server（stdio）。
 *
 * 手写 JSON-RPC 而不是引第三方包，理由：
 *   1. 这个 server 能执行任意 shell 命令，是整个工具链里权限最高的一环。
 *      代码放在仓库里、只有两百行，任何人都能读完确认它做了什么。
 *   2. 不引依赖就没有供应链面 —— npx 拉一个来源不明的包来跑 shell，
 *      风险比它解决的问题大。
 *   3. 协议形状和 Anima 的闸门是配套设计的（见下面 open_app 与 run_command 的拆分）。
 *
 * ## 为什么把 open_app 和 run_command 拆成两个工具
 *
 * Anima 的闸门按**工具名**分类、按**参数**查危险模式。如果只有 run_command，
 * 那么"打开终端"这种无害操作也得走 Execute 的确认流程，用起来很烦；
 * 而把 open 塞进白名单又等于放行 `open` 的全部用法。
 *
 * 拆开之后 open_app 的参数只能是应用名，构造不出任意命令，所以可以单独放行；
 * run_command 保持"白名单外一律确认"。
 */

import { spawn } from "node:child_process";

const send = (msg) => process.stdout.write(JSON.stringify(msg) + "\n");
const ok = (id, result) => send({ jsonrpc: "2.0", id, result });
const failText = (id, text) =>
  ok(id, { content: [{ type: "text", text }], isError: true });

/** 单条命令的墙钟上限。超时必须杀掉进程组，否则子进程会留下来变成孤儿 */
const TIMEOUT_MS = 20_000;
/** 回给模型的输出上限。几十万行日志会直接把上下文吃光 */
const MAX_OUTPUT = 8_000;

const TOOLS = [
  {
    name: "open_app",
    description:
      "打开一个 macOS 应用程序（等价于 open -a）。参数只能是应用名，" +
      "无法构造任意命令，因此比 run_command 安全得多。",
    inputSchema: {
      type: "object",
      properties: {
        app: { type: "string", description: "应用名，如 Terminal、Safari、Finder" },
      },
      required: ["app"],
    },
  },
  {
    name: "run_command",
    description:
      "在 shell 里执行一条命令并返回其标准输出与标准错误。" +
      "能执行任意命令，属于最高风险的工具。" +
      "禁止用它调用 afplay/mpv/ffplay 播放音频；音频播放必须走 Anima 的 play_audio 工具。",
    inputSchema: {
      type: "object",
      properties: {
        command: { type: "string", description: "要执行的完整命令行" },
        cwd: { type: "string", description: "工作目录，缺省为用户主目录" },
      },
      required: ["command"],
    },
  },
  {
    name: "get_system_info",
    description: "读取系统概况：主机名、系统版本、CPU、内存、磁盘占用、运行时长。",
    inputSchema: { type: "object", properties: {} },
  },
];

/** 跑一条命令，返回 {code, out}。永不 reject —— 失败信息要作为文本回给模型 */
function run(command, cwd) {
  return new Promise((resolve) => {
    const child = spawn("/bin/zsh", ["-lc", command], {
      cwd: cwd || process.env.HOME,
      // detached 让它自成进程组，超时时能整组杀掉；
      // 不这么做的话 `cmd | other` 里的 other 会活下来
      detached: true,
      stdio: ["ignore", "pipe", "pipe"],
    });

    let out = "";
    const push = (chunk) => {
      if (out.length < MAX_OUTPUT) out += chunk;
    };
    child.stdout.on("data", push);
    child.stderr.on("data", push);

    const timer = setTimeout(() => {
      try {
        process.kill(-child.pid, "SIGKILL");
      } catch {
        /* 进程已经退了 */
      }
      resolve({ code: null, out: out + `\n[超时 ${TIMEOUT_MS / 1000}s，已终止]` });
    }, TIMEOUT_MS);

    child.on("error", (err) => {
      clearTimeout(timer);
      resolve({ code: -1, out: `启动失败：${err.message}` });
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      const trimmed =
        out.length > MAX_OUTPUT
          ? out.slice(0, MAX_OUTPUT) + `\n[输出已截断，共 ${out.length} 字符]`
          : out;
      resolve({ code, out: trimmed });
    });
  });
}

function commandUsesSystemAudioPlayer(command) {
  for (const raw of String(command).split(/[^A-Za-z0-9_./+-]+/)) {
    if (!raw) continue;
    const executable = raw.split("/").pop();
    if (executable === "afplay" || executable === "mpv" || executable === "ffplay") {
      return executable;
    }
  }
  return null;
}

async function callTool(name, args) {
  if (name === "open_app") {
    const app = String(args.app ?? "").trim();
    if (!app) return { text: "缺少 app 参数", isError: true };
    // 只允许应用名：出现路径分隔符或 shell 元字符就拒绝，
    // 否则 open_app 会变成一个绕过 run_command 确认流程的后门
    if (/[;&|`$(){}<>\n\\/]/.test(app)) {
      return {
        text: `应用名里不允许出现路径或 shell 元字符：${app}`,
        isError: true,
      };
    }
    // 先按原名试。英文名走这条，最快
    const direct = await run(`open -a ${JSON.stringify(app)}`);
    if (direct.code === 0) return { text: `已打开 ${app}` };

    // **中文名必须能用。**
    //
    // 用户说的是"打开终端"，模型自然会传「终端」，而 `open -a 终端` 会失败
    // （`Unable to find application named '终端'`）。实测撞到过：工具返回了
    // 错误、模型却仍然回答"已经打开了"。
    //
    // 靠模型自己纠正成 Terminal 不可靠（小模型的工具调用本来就弱）。所以在这里
    // 解决：用 Spotlight 按**本地化显示名**查真实的 .app 路径。
    // 那是系统自己维护的索引，比在这里硬编码一张中英对照表通用得多 ——
    // 终端 / 访达 / 系统设置 / 计算器 都能查到。
    const resolved = await run(
      `mdfind ${JSON.stringify(
        `kMDItemContentType == 'com.apple.application-bundle' && kMDItemDisplayName == '${app.replace(/'/g, "")}'`,
      )} | head -1`,
    );
    const path = resolved.out.trim().split("\n")[0] ?? "";
    if (path.endsWith(".app")) {
      const byPath = await run(`open ${JSON.stringify(path)}`);
      if (byPath.code === 0) {
        // 把真实名字一并告诉模型，它下次就会直接用对的那个
        const real = path.split("/").pop().replace(/\.app$/, "");
        return { text: `已打开 ${app}（实际应用名：${real}）` };
      }
    }

    // 两条都失败时**说清楚试过什么** —— 只回"失败"的话模型无从修正，
    // 而它的下一步往往是直接宣布成功
    return {
      text:
        `打开「${app}」失败。已尝试：open -a "${app}"（${direct.out.trim() || "无输出"}）、` +
        `以及按本地化名查找（${path || "没找到匹配的应用"}）。` +
        `请换成应用的英文名重试，例如 Terminal / Finder / Safari。`,
      isError: true,
    };
  }

  if (name === "run_command") {
    const command = String(args.command ?? "").trim();
    if (!command) return { text: "缺少 command 参数", isError: true };
    const player = commandUsesSystemAudioPlayer(command);
    if (player) {
      return {
        text:
          `禁止通过 system__run_command 调用 ${player} 播放音频。` +
          "请改用 Anima 专用 play_audio 工具，并传入 url_or_path。",
        isError: true,
      };
    }
    const { code, out } = await run(command, args.cwd);
    const body = out.trim() || "（无输出）";
    return code === 0
      ? { text: body }
      : { text: `退出码 ${code}\n${body}`, isError: true };
  }

  if (name === "get_system_info") {
    const { out } = await run(
      "echo \"主机: $(scutil --get ComputerName 2>/dev/null || hostname)\"; " +
        "echo \"系统: $(sw_vers -productName) $(sw_vers -productVersion)\"; " +
        "echo \"CPU: $(sysctl -n machdep.cpu.brand_string)\"; " +
        "echo \"内存: $(( $(sysctl -n hw.memsize) / 1073741824 )) GB\"; " +
        "echo \"运行: $(uptime | sed 's/.*up //; s/,.*users.*//')\"; " +
        "df -h / | tail -1 | awk '{print \"磁盘: \"$3\" / \"$2\" 已用 \"$5}'",
    );
    return { text: out.trim() || "读取失败" };
  }

  return { text: `不支持的工具：${name}`, isError: true };
}

let buf = "";
process.stdin.on("data", async (chunk) => {
  buf += chunk;
  let idx;
  while ((idx = buf.indexOf("\n")) >= 0) {
    const line = buf.slice(0, idx).trim();
    buf = buf.slice(idx + 1);
    if (!line) continue;

    let req;
    try {
      req = JSON.parse(line);
    } catch {
      continue;
    }
    // 通知没有 id，不需要回应（notifications/initialized 走这条）
    if (req.id === undefined || req.id === null) continue;

    if (req.method === "initialize") {
      ok(req.id, {
        protocolVersion: req.params?.protocolVersion ?? "2024-11-05",
        capabilities: { tools: {} },
        serverInfo: { name: "anima-system", version: "0.1.0" },
      });
    } else if (req.method === "tools/list") {
      ok(req.id, { tools: TOOLS });
    } else if (req.method === "tools/call") {
      try {
        const r = await callTool(req.params?.name, req.params?.arguments ?? {});
        ok(req.id, {
          content: [{ type: "text", text: r.text }],
          ...(r.isError ? { isError: true } : {}),
        });
      } catch (err) {
        // 工具报错要作为内容回给模型，而不是 JSON-RPC error —— 模型看到原因
        // 才能自我修正，抛协议错它只会看到一次失败
        failText(req.id, `执行时异常：${err?.message ?? err}`);
      }
    } else {
      send({
        jsonrpc: "2.0",
        id: req.id,
        error: { code: -32601, message: `未实现的方法：${req.method}` },
      });
    }
  }
});
