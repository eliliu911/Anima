//! MCP 服务端协议层：JSON-RPC 2.0 over stdio。
//!
//! **这一层不碰工具、不碰文件、不碰网络** —— 它只把一行 JSON 变成另一行 JSON，
//! 工具那头藏在 [`Tools`] 后面。这样协议本身可以完全离线测，
//! 而协议错误（少个字段、方法名写错、错误码不对）恰恰是最难在真连接里看出来的：
//! 客户端通常只显示「服务器无响应」。
//!
//! # 分帧：**一行一个 JSON**
//!
//! MCP 的 stdio 传输是换行分隔的 JSON-RPC，不是 LSP 那种
//! `Content-Length` 头。写错的后果是客户端一个字节都读不懂，
//! 而两边都不会报「分帧错了」。
//!
//! # 通知没有回复
//!
//! `notifications/initialized` 这类**没有 `id`**，按 JSON-RPC 的规矩
//! 不能回。回了的话严格的客户端会报协议错误。

use serde::Deserialize;
use serde_json::{json, Value};

/// 报给客户端的协议版本。客户端会拿它和自己支持的比。
pub const PROTOCOL_VERSION: &str = "2025-06-18";

pub const SERVER_NAME: &str = "anima";

/// JSON-RPC 的标准错误码。**别自己编** —— 客户端按码分支。
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const PARSE_ERROR: i32 = -32700;

#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

/// 一个能列工具、能调工具的东西。**协议层只认这个**。
#[async_trait::async_trait]
pub trait Tools: Send + Sync {
    /// `(名字, 说明, JSON Schema)`
    fn list(&self) -> Vec<(String, String, Value)>;
    /// 回 `(正文, 是不是错误)`
    async fn call(&self, name: &str, args: Value) -> (String, bool);
}

fn ok(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn err(id: Option<Value>, code: i32, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
}

/// 处理一条请求。回 `None` 表示**这是通知，不该回复**。
pub async fn handle(req: Request, tools: &dyn Tools, version: &str) -> Option<Value> {
    // 通知：没有 id。**一律不回** —— 回了严格的客户端会报协议错误
    if req.id.is_none() {
        return None;
    }
    let id = req.id.clone();
    Some(match req.method.as_str() {
        "initialize" => ok(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                // 只声明 tools。**声明了却没实现的能力，客户端会去调然后拿到 -32601**
                "capabilities": { "tools": {} },
                "serverInfo": { "name": SERVER_NAME, "version": version },
            }),
        ),
        "ping" => ok(id, json!({})),
        "tools/list" => ok(
            id,
            json!({
                "tools": tools
                    .list()
                    .into_iter()
                    .map(|(name, description, input_schema)| json!({
                        "name": name,
                        "description": description,
                        "inputSchema": input_schema,
                    }))
                    .collect::<Vec<_>>()
            }),
        ),
        "tools/call" => {
            let params = req.params.unwrap_or(Value::Null);
            let Some(name) = params.get("name").and_then(|v| v.as_str()) else {
                return Some(err(id, INVALID_PARAMS, "tools/call 缺 name"));
            };
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let (text, is_error) = tools.call(name, args).await;
            // **工具失败不是 JSON-RPC 错误。** MCP 要求它作为正常结果回去、
            // 带 isError=true —— 报成 JSON-RPC error 的话客户端会当成传输故障，
            // 模型看不到失败原因、也就没法改参数重试。
            ok(id, json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }))
        }
        other => Some(err(id, METHOD_NOT_FOUND, format!("不支持的方法：{other}")))?,
    })
}

/// 解析一行。解析不了时回一条 JSON-RPC 解析错误（`id` 为 null）。
pub fn parse_line(line: &str) -> Result<Request, Value> {
    serde_json::from_str::<Request>(line)
        .map_err(|e| err(None, PARSE_ERROR, format!("这行不是合法的 JSON-RPC：{e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;

    #[async_trait::async_trait]
    impl Tools for Fake {
        fn list(&self) -> Vec<(String, String, Value)> {
            vec![("t".into(), "说明".into(), json!({"type":"object"}))]
        }
        async fn call(&self, name: &str, args: Value) -> (String, bool) {
            if name == "boom" {
                ("炸了".into(), true)
            } else {
                (format!("{name}:{args}"), false)
            }
        }
    }

    async fn go(v: Value) -> Option<Value> {
        handle(serde_json::from_value(v).unwrap(), &Fake, "9.9.9").await
    }

    #[tokio::test]
    async fn 握手回协议版本和服务名() {
        let r = go(json!({"jsonrpc":"2.0","id":1,"method":"initialize"})).await.unwrap();
        assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(r["result"]["serverInfo"]["name"], SERVER_NAME);
        assert_eq!(r["result"]["serverInfo"]["version"], "9.9.9");
        // **只声明 tools** —— 声明了没实现的能力，客户端会去调然后拿到 -32601
        assert!(r["result"]["capabilities"].get("tools").is_some());
        assert_eq!(r["result"]["capabilities"].as_object().unwrap().len(), 1);
    }

    /// **通知不能回。** 回了严格的客户端会报协议错误。
    #[tokio::test]
    async fn 通知不回复() {
        assert!(go(json!({"jsonrpc":"2.0","method":"notifications/initialized"})).await.is_none());
        assert!(go(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{}})).await.is_none());
    }

    #[tokio::test]
    async fn 列工具用_inputSchema_这个键名() {
        let r = go(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).await.unwrap();
        let t = &r["result"]["tools"][0];
        // **键名是 inputSchema（小驼峰）。** 写成 input_schema 的话客户端
        // 认不出参数，表现是「工具有，但调用时永远不传参数」
        assert!(t.get("inputSchema").is_some(), "键名写错了：{t}");
        assert_eq!(t["name"], "t");
    }

    /// **工具失败要作为正常结果回去**，不是 JSON-RPC error ——
    /// 报成 error 客户端会当传输故障，模型看不到原因、没法改参数重试。
    #[tokio::test]
    async fn 工具失败不报成协议错误() {
        let r = go(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"boom"}})).await.unwrap();
        assert!(r.get("error").is_none(), "工具失败被报成了协议错误：{r}");
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(r["result"]["content"][0]["text"], "炸了");
    }

    #[tokio::test]
    async fn 调用缺名字是参数错误() {
        let r = go(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{}})).await.unwrap();
        assert_eq!(r["error"]["code"], INVALID_PARAMS);
    }

    #[tokio::test]
    async fn 没给参数时按空对象传() {
        let r = go(json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"t"}})).await.unwrap();
        assert_eq!(r["result"]["content"][0]["text"], "t:{}");
    }

    #[tokio::test]
    async fn 不认识的方法回_32601() {
        let r = go(json!({"jsonrpc":"2.0","id":6,"method":"resources/list"})).await.unwrap();
        assert_eq!(r["error"]["code"], METHOD_NOT_FOUND);
    }

    #[test]
    fn 坏行回解析错误而不是崩() {
        let e = parse_line("{ 这不是 json").unwrap_err();
        assert_eq!(e["error"]["code"], PARSE_ERROR);
        assert_eq!(e["id"], Value::Null);
        assert_eq!(e["jsonrpc"], "2.0");
    }

    /// 每条回复都必须带 `jsonrpc: "2.0"` 和原样的 id。
    #[tokio::test]
    async fn 回复带版本和原样的id() {
        for (id, method) in [(json!(7), "ping"), (json!("abc"), "tools/list")] {
            let r = go(json!({"jsonrpc":"2.0","id":id,"method":method})).await.unwrap();
            assert_eq!(r["jsonrpc"], "2.0");
            assert_eq!(r["id"], id);
        }
    }
}
