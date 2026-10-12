//! MCP stdio 传输的端到端测试。
//!
//! 自带一个最小 MCP server（Node 手写 JSON-RPC，不依赖任何包，也不联网），
//! 真的起子进程、真的走 initialize 握手、真的列工具和调工具。
//!
//! 为什么值得这么测：`cargo check` 通过只说明类型对得上。MCP 的握手、
//! 协议版本、`tools/list` 的字段形状、`tools/call` 的返回结构，
//! 任何一处对不上都要到运行时才炸 —— 而那时候用户已经在等一个不会来的回答了。

use std::collections::BTreeMap;

use anima_lib::mcp::config::{McpServerConfig, McpTransport};
use anima_lib::mcp::guard::{ApprovalMode, CommandPolicy};
use anima_lib::mcp::permission::{Access, ServerPermissions};
use anima_lib::mcp::{Authorization, McpRegistry};

/// 一个够用的 MCP server：initialize + tools/list + tools/call。
///
/// 导出三个工具，刚好覆盖闸门的三条路径：
///   - `read_note`   → 读，直接放行
///   - `delete_note` → 删，默认禁用（连声明都不该出现）
///   - `run_command` → 执行，风险在参数里
const SERVER_JS: &str = r#"
const send = (msg) => process.stdout.write(JSON.stringify(msg) + "\n");
const TOOLS = [
  { name: "read_note", description: "读取一条笔记",
    inputSchema: { type: "object", properties: { id: { type: "string" } } } },
  { name: "delete_note", description: "删除一条笔记",
    inputSchema: { type: "object", properties: { id: { type: "string" } } } },
  { name: "run_command", description: "在 shell 里运行命令",
    inputSchema: { type: "object", properties: { command: { type: "string" } } } },
];

let buf = "";
process.stdin.on("data", (chunk) => {
  buf += chunk;
  let idx;
  while ((idx = buf.indexOf("\n")) >= 0) {
    const line = buf.slice(0, idx).trim();
    buf = buf.slice(idx + 1);
    if (!line) continue;
    let req;
    try { req = JSON.parse(line); } catch { continue; }
    // 通知没有 id，不需要回应
    if (req.id === undefined || req.id === null) continue;

    if (req.method === "initialize") {
      send({ jsonrpc: "2.0", id: req.id, result: {
        protocolVersion: req.params?.protocolVersion ?? "2024-11-05",
        capabilities: { tools: {} },
        serverInfo: { name: "anima-test-server", version: "0.0.1" },
      }});
    } else if (req.method === "tools/list") {
      send({ jsonrpc: "2.0", id: req.id, result: { tools: TOOLS } });
    } else if (req.method === "tools/call") {
      const name = req.params?.name;
      const args = req.params?.arguments ?? {};
      if (name === "read_note") {
        send({ jsonrpc: "2.0", id: req.id, result: {
          content: [{ type: "text", text: "笔记内容：" + (args.id ?? "?") }],
        }});
      } else if (name === "run_command") {
        send({ jsonrpc: "2.0", id: req.id, result: {
          content: [{ type: "text", text: "已运行：" + (args.command ?? "") }],
        }});
      } else {
        // 工具自身报错：MCP 用 isError 而不是 JSON-RPC error
        send({ jsonrpc: "2.0", id: req.id, result: {
          content: [{ type: "text", text: "不支持的工具：" + name }],
          isError: true,
        }});
      }
    } else {
      send({ jsonrpc: "2.0", id: req.id, error: { code: -32601, message: "未实现" } });
    }
  }
});
"#;

fn write_server() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("anima-mcp-test");
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let path = dir.join("server.mjs");
    std::fs::write(&path, SERVER_JS).expect("写测试 server");
    path
}

fn config(script: &std::path::Path, permissions: ServerPermissions) -> McpServerConfig {
    McpServerConfig {
        id: "test".into(),
        label: "测试 server".into(),
        enabled: true,
        transport: McpTransport::Stdio {
            command: "node".into(),
            args: vec![script.to_string_lossy().to_string()],
            env: BTreeMap::new(),
            env_from_secrets: BTreeMap::new(),
        },
        permissions,
        approval_mode: ApprovalMode::Auto,
        commands: CommandPolicy::default(),
        startup_timeout_secs: 20,
    }
}

/// 一次跑完：连接 → 列工具 → 三条闸门路径 → 真调一次。
///
/// 合在一个测试里是有意的 —— 起子进程有开销，而这些断言共享同一个连接，
/// 拆开只会让测试变慢，并不会让失败定位更清楚（每个断言都有自己的说明）。
#[tokio::test]
async fn stdio_server_connects_lists_and_calls_through_the_guard() {
    if which_node().is_none() {
        eprintln!("跳过：本机没有 node");
        return;
    }
    let script = write_server();
    let registry = McpRegistry::connect(
        &[config(&script, ServerPermissions::default())],
        &BTreeMap::new(),
    )
    .await;

    // ── 连接 ──
    let status = &registry.statuses()[0];
    assert!(
        status.connected,
        "应当连上测试 server，实际错误：{:?}",
        status.error
    );
    assert_eq!(status.tool_count, 3, "server 导出了 3 个工具");

    // ── 声明过滤：delete 默认禁用，连声明都不该出现 ──
    let declared: Vec<String> = registry
        .tool_declarations()
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert!(declared.contains(&"test__read_note".to_string()));
    assert!(declared.contains(&"test__run_command".to_string()));
    assert!(
        !declared.contains(&"test__delete_note".to_string()),
        "删除类工具默认禁用，不该出现在声明里：{declared:?}"
    );
    // 但界面上要能看到它（用户得知道有这个工具、且它为什么被禁）
    assert_eq!(registry.describe_tools().len(), 3);

    // ── 读：直接放行，且真的调得通 ──
    let auth = registry
        .authorize("test__read_note", r#"{"id":"n1"}"#)
        .expect("授权不该报错");
    let approved = match auth {
        Authorization::Ready(call) => call,
        other => panic!("读操作应当直接放行，实际：{other:?}"),
    };
    let (text, _) = registry.execute(approved).await.expect("调用应当成功");
    assert_eq!(text, "笔记内容：n1");

    // ── 执行：非白名单命令要确认，确认后能真的执行 ──
    let auth = registry
        .authorize("test__run_command", r#"{"command":"brew list"}"#)
        .expect("授权不该报错");
    let approved = match auth {
        Authorization::NeedsConfirmation { call, verdict } => {
            assert!(
                verdict.reason().contains("brew list"),
                "确认说明里要带上具体命令，实际：{}",
                verdict.reason()
            );
            call.approve()
        }
        other => panic!("非白名单命令应当需要确认，实际：{other:?}"),
    };
    let (text, _) = registry.execute(approved).await.expect("确认后应当能执行");
    assert_eq!(text, "已运行：brew list");

    // ── 白名单命令免问 ──
    assert!(
        matches!(
            registry.authorize("test__run_command", r#"{"command":"git status"}"#),
            Ok(Authorization::Ready(_))
        ),
        "白名单里的命令应当免问"
    );

    // ── 高危：Auto 模式也拦住，且拿不到可执行的调用 ──
    match registry
        .authorize(
            "test__run_command",
            r#"{"command":"diskutil eraseDisk JHFS+ X disk0"}"#,
        )
        .expect("授权不该报错")
    {
        Authorization::Rejected(verdict) => assert!(
            verdict.reason().contains("抹除磁盘"),
            "要说清拒绝原因，实际：{}",
            verdict.reason()
        ),
        other => panic!("抹盘必须被拒绝，实际：{other:?}"),
    }

    registry.shutdown().await;
}

/// 工具自身报错时，内容要回填给模型而不是变成 Err ——
/// 模型看到错误才能自我修正。
#[tokio::test]
async fn tool_level_errors_come_back_as_text() {
    if which_node().is_none() {
        return;
    }
    let script = write_server();
    // 把 delete 放开到 Allow，好让请求真的打到 server（它会回 isError）
    let permissions = ServerPermissions {
        delete: Access::Allow,
        ..Default::default()
    };
    let registry = McpRegistry::connect(&[config(&script, permissions)], &BTreeMap::new()).await;
    assert!(registry.statuses()[0].connected);

    let auth = registry
        .authorize("test__delete_note", r#"{"id":"n1"}"#)
        .unwrap();
    let approved = match auth {
        Authorization::Ready(c) => c,
        other => panic!("已放开为 Allow，应当直接可执行：{other:?}"),
    };
    let (text, _) = registry.execute(approved).await.expect("不该变成 Err");
    assert!(
        text.contains("工具执行出错"),
        "工具报错要作为文本回填，实际：{text}"
    );
    registry.shutdown().await;
}

/// 起不来的 server 不该让整个注册表失败，且错误原因要留下来。
#[tokio::test]
async fn a_broken_server_does_not_break_the_registry() {
    let mut broken = config(
        std::path::Path::new("/nonexistent/nope.mjs"),
        ServerPermissions::default(),
    );
    broken.id = "broken".into();
    if let McpTransport::Stdio { command, .. } = &mut broken.transport {
        *command = "definitely-not-a-real-command-xyz".into();
    }

    let registry = McpRegistry::connect(&[broken], &BTreeMap::new()).await;
    let status = &registry.statuses()[0];
    assert!(!status.connected);
    assert!(
        status.error.is_some(),
        "连不上就必须留下原因，否则用户没法排查"
    );
    // 没有可用工具，但注册表本身仍然可用
    assert!(registry.tool_declarations().is_empty());
}

fn which_node() -> Option<()> {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| ())
}
