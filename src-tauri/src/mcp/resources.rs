use crate::error::{Error, Result};
use crate::mcp::McpRegistry;
use serde_json::{json, Value};

pub async fn list_resources(registry: &McpRegistry) -> Result<Vec<Value>> {
    let mut all_resources = Vec::new();
    for (server_id, server) in &registry.servers {
        if let Some(client) = &server.client {
            if let Ok(Value::Array(resources)) = client.list_resources().await {
                for mut res in resources {
                    if let Some(obj) = res.as_object_mut() {
                        obj.insert("server_id".to_string(), json!(server_id));
                    }
                    all_resources.push(res);
                }
            }
        }
    }
    Ok(all_resources)
}

pub async fn read_resource(registry: &McpRegistry, server_id: &str, uri: &str) -> Result<Value> {
    let server = registry
        .servers
        .get(server_id)
        .ok_or_else(|| Error::config("Server not found"))?;
    let client = server
        .client
        .as_ref()
        .ok_or_else(|| Error::config("Server not connected"))?;
    client.read_resource(uri).await
}
