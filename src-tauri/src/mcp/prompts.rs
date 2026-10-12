use crate::error::{Error, Result};
use crate::mcp::McpRegistry;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub async fn list_prompts(registry: &McpRegistry) -> Result<Vec<Value>> {
    let mut all_prompts = Vec::new();
    for (server_id, server) in &registry.servers {
        if let Some(client) = &server.client {
            if let Ok(Value::Array(prompts)) = client.list_prompts().await {
                for mut prompt in prompts {
                    if let Some(obj) = prompt.as_object_mut() {
                        obj.insert("server_id".to_string(), json!(server_id));
                    }
                    all_prompts.push(prompt);
                }
            }
        }
    }
    Ok(all_prompts)
}

pub async fn get_prompt(
    registry: &McpRegistry,
    server_id: &str,
    name: &str,
    args: Option<BTreeMap<String, String>>,
) -> Result<Value> {
    let server = registry
        .servers
        .get(server_id)
        .ok_or_else(|| Error::config("Server not found"))?;
    let client = server
        .client
        .as_ref()
        .ok_or_else(|| Error::config("Server not connected"))?;
    client.get_prompt(name, args).await
}
