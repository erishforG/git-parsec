//! Stdio JSON-RPC 2.0 server for `parsec mcp serve` (Phase 2, issue #293).
//!
//! Reads newline-delimited JSON from stdin, dispatches MCP method calls,
//! and writes JSON-RPC 2.0 responses to stdout.  Each response is a single
//! line (compact JSON) followed by a newline so Claude Desktop / Cursor can
//! frame messages without a length prefix.
//!
//! ## Supported methods (Phase 2)
//!
//! | Method          | Behaviour                                              |
//! |-----------------|--------------------------------------------------------|
//! | `initialize`    | Respond with server info + capabilities.               |
//! | `tools/list`    | Return the TOOLS catalogue from `crate::mcp::TOOLS`.  |
//! | `tools/call`    | Stub — returns "not implemented" until Phase 3.        |
//! | `ping`          | Echo `{"pong": true}`.                                 |
//! | *(other)*       | JSON-RPC -32601 method-not-found error.                |
//!
//! ## Phase 3 hook
//!
//! Replace the `tools/call` stub arm with real dispatch to
//! `crate::mcp::tools::*::handle(ctx, params)`.

use std::io::{BufRead, Write};

use anyhow::Result;
use serde_json::{json, Value};

use crate::mcp::{McpContext, TOOLS};

/// MCP server name & version embedded in `initialize` responses.
const SERVER_NAME: &str = "git-parsec";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Stdio JSON-RPC 2.0 server.
pub struct McpServer {
    ctx: McpContext,
}

impl McpServer {
    /// Create a server bound to the given context.
    pub fn new(ctx: McpContext) -> Self {
        Self { ctx }
    }

    /// Run the read-dispatch-write loop until stdin closes.
    ///
    /// Each line on stdin is treated as one JSON-RPC request.  Blank lines
    /// are silently skipped.  Parse errors produce a `-32700` parse error
    /// response (if we can extract a request `id`, we use it; otherwise `null`).
    ///
    /// # Errors
    ///
    /// Returns an error only if stdout becomes unwritable.  Malformed input
    /// and unknown methods are handled inline as JSON-RPC error responses.
    pub fn serve(self) -> Result<()> {
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        let mut out = stdout.lock();

        for line in stdin.lock().lines() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let response = match serde_json::from_str::<Value>(trimmed) {
                Ok(req) => self.dispatch(&req),
                Err(_) => rpc_error(Value::Null, -32700, "Parse error", None),
            };

            serde_json::to_writer(&mut out, &response)?;
            out.write_all(b"\n")?;
            out.flush()?;
        }

        Ok(())
    }

    /// Dispatch one parsed JSON-RPC request and return the response value.
    fn dispatch(&self, req: &Value) -> Value {
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(Value::Null);

        match method {
            "initialize" => self.handle_initialize(id, &params),
            "tools/list" => self.handle_tools_list(id),
            "tools/call" => self.handle_tools_call(id, &params),
            "ping" => rpc_ok(id, json!({"pong": true})),
            "" => rpc_error(id, -32600, "Invalid Request: missing method", None),
            other => rpc_error(id, -32601, &format!("Method not found: {other}"), None),
        }
    }

    // ── Method handlers ──────────────────────────────────────────────────────

    fn handle_initialize(&self, id: Value, _params: &Value) -> Value {
        rpc_ok(
            id,
            json!({
                "protocolVersion": "2024-11-05",
                "serverInfo": {
                    "name": SERVER_NAME,
                    "version": SERVER_VERSION,
                },
                "capabilities": {
                    "tools": {}
                }
            }),
        )
    }

    fn handle_tools_list(&self, id: Value) -> Value {
        let tools: Vec<Value> = TOOLS
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    // Phase 3 will add a real `inputSchema`; for now emit
                    // an empty object schema so clients accept the response.
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                })
            })
            .collect();

        rpc_ok(id, json!({ "tools": tools }))
    }

    fn handle_tools_call(&self, id: Value, params: &Value) -> Value {
        let tool_name = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>");

        // Phase 3: replace this stub with real dispatch once tool handlers
        // are implemented in crate::mcp::tools.
        let _ = &self.ctx; // ctx will be used in Phase 3
        rpc_error(
            id,
            -32603,
            &format!("Tool '{tool_name}' not yet implemented (Phase 3, issue #293)"),
            None,
        )
    }
}

// ── JSON-RPC 2.0 helpers ─────────────────────────────────────────────────────

fn rpc_ok(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn rpc_error(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut err = json!({
        "code": code,
        "message": message,
    });
    if let Some(d) = data {
        err["data"] = d;
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": err,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn server() -> McpServer {
        let ctx = McpContext::from_cwd(false).expect("McpContext");
        McpServer::new(ctx)
    }

    #[test]
    fn initialize_returns_protocol_version() {
        let resp = server().dispatch(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        }));
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(resp["result"]["serverInfo"]["name"], "git-parsec");
    }

    #[test]
    fn tools_list_returns_all_tools() {
        let resp = server().dispatch(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }));
        let tools = resp["result"]["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), TOOLS.len());
        // Each entry must have name + description + inputSchema
        for t in tools {
            assert!(t["name"].is_string());
            assert!(t["description"].is_string());
            assert!(t["inputSchema"].is_object());
        }
    }

    #[test]
    fn tools_call_returns_not_implemented() {
        let resp = server().dispatch(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "worktree_list", "arguments": {} }
        }));
        assert!(resp["error"].is_object());
        assert_eq!(resp["error"]["code"], -32603_i64);
    }

    #[test]
    fn ping_returns_pong() {
        let resp = server().dispatch(&json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "ping"
        }));
        assert_eq!(resp["result"]["pong"], true);
    }

    #[test]
    fn unknown_method_returns_method_not_found() {
        let resp = server().dispatch(&json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "no_such_method"
        }));
        assert_eq!(resp["error"]["code"], -32601_i64);
    }

    #[test]
    fn missing_id_uses_null() {
        let resp = server().dispatch(&json!({
            "jsonrpc": "2.0",
            "method": "ping"
        }));
        assert!(resp["id"].is_null());
        assert_eq!(resp["result"]["pong"], true);
    }
}
