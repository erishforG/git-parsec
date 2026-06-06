//! stdio JSON-RPC 2.0 server for `parsec mcp serve`.
//!
//! ## Phase 2 (#293) — transport + method dispatch
//!
//! Implements the MCP stdio transport loop and dispatches these methods:
//!
//! | Method         | Status   | Notes                                   |
//! |----------------|----------|-----------------------------------------|
//! | `initialize`   | ✅ real   | returns server info + capabilities      |
//! | `tools/list`   | ✅ real   | returns full TOOLS registry             |
//! | `tools/call`   | 🔧 stub  | echoes "Phase 3" until handlers land    |
//! | `shutdown`     | ✅ real   | returns null result, server keeps going |
//!
//! Phase 3 (#293) will replace the `tools/call` stub with real handler
//! dispatch into `src/mcp/tools/`.
//!
//! ## Transport
//!
//! Reads newline-delimited JSON-RPC 2.0 from **stdin**, writes
//! newline-delimited responses to **stdout** (standard MCP stdio transport).
//! One request per line; the server terminates on stdin EOF.

use anyhow::Result;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

use crate::mcp::TOOLS;

/// JSON-RPC 2.0 standard error codes.
mod code {
    pub const PARSE_ERROR: i32 = -32700;
    pub const METHOD_NOT_FOUND: i32 = -32601;
}

/// Run the MCP stdio server loop (blocking).
///
/// Reads newline-delimited JSON from stdin until EOF, writes JSON responses
/// to stdout. Intended to be called from [`tokio::task::spawn_blocking`].
///
/// # Errors
/// Returns an error only on I/O failure (broken pipe, etc.).
pub fn run() -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());

    for line in stdin.lock().lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Value>(trimmed) {
            Ok(req) => dispatch(&req),
            Err(e) => json_error(Value::Null, code::PARSE_ERROR, &format!("Parse error: {e}")),
        };

        serde_json::to_writer(&mut out, &response)?;
        out.write_all(b"\n")?;
        out.flush()?;
    }
    Ok(())
}

// ── dispatch ────────────────────────────────────────────────────────────────

fn dispatch(req: &Value) -> Value {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(json!({}));

    match method {
        "initialize" => handle_initialize(id, &params),
        "tools/list" => handle_tools_list(id),
        "tools/call" => handle_tools_call(id, &params),
        "shutdown" => json_ok(id, Value::Null),
        "" => json_error(id, code::PARSE_ERROR, "Missing 'method' field"),
        other => json_error(
            id,
            code::METHOD_NOT_FOUND,
            &format!("Method not found: '{other}'"),
        ),
    }
}

// ── method handlers ──────────────────────────────────────────────────────────

fn handle_initialize(id: Value, _params: &Value) -> Value {
    json_ok(
        id,
        json!({
            "protocolVersion": "2024-11-05",
            "serverInfo": {
                "name": "git-parsec",
                "version": env!("CARGO_PKG_VERSION")
            },
            "capabilities": {
                "tools": {}
            }
        }),
    )
}

fn handle_tools_list(id: Value) -> Value {
    let tools: Vec<Value> = TOOLS
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            })
        })
        .collect();

    json_ok(id, json!({ "tools": tools }))
}

/// Phase 2 stub: acknowledges known tools, returns "not implemented" for all.
///
/// Phase 3 will replace this with real handler dispatch into `src/mcp/tools/`.
fn handle_tools_call(id: Value, params: &Value) -> Value {
    let name = params
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("<unknown>");

    let known = TOOLS.iter().any(|t| t.name == name);
    if !known {
        return json_error(
            id,
            code::METHOD_NOT_FOUND,
            &format!("Unknown tool: '{name}'"),
        );
    }

    // Phase 3 will dispatch to real tool handlers in src/mcp/tools/.
    json_ok(
        id,
        json!({
            "content": [{
                "type": "text",
                "text": format!("Tool '{name}' not yet implemented — lands in Phase 3 (Refs #293)")
            }],
            "isError": true
        }),
    )
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn json_ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn json_error(id: Value, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_returns_protocol_version_and_server_info() {
        let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} });
        let resp = dispatch(&req);
        let result = &resp["result"];
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert!(result["serverInfo"]["name"].as_str().is_some());
        assert!(result["serverInfo"]["version"].as_str().is_some());
        assert!(result["capabilities"]["tools"].is_object());
    }

    #[test]
    fn tools_list_returns_all_registered_tools() {
        let req = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        let resp = dispatch(&req);
        let tools = resp["result"]["tools"].as_array().expect("tools array");
        assert!(!tools.is_empty(), "TOOLS registry must not be empty");
        for tool in tools {
            assert!(tool["name"].as_str().is_some(), "tool must have name");
            assert!(
                tool["description"].as_str().is_some(),
                "tool must have description"
            );
            assert!(
                tool["inputSchema"].is_object(),
                "tool must have inputSchema"
            );
        }
    }

    #[test]
    fn tools_call_unknown_tool_returns_method_not_found() {
        let req = json!({
            "jsonrpc": "2.0", "id": 3,
            "method": "tools/call",
            "params": { "name": "nonexistent_tool" }
        });
        let resp = dispatch(&req);
        assert!(
            resp.get("error").is_some(),
            "expected error for unknown tool"
        );
        assert_eq!(resp["error"]["code"], code::METHOD_NOT_FOUND);
    }

    #[test]
    fn tools_call_known_tool_returns_phase3_stub() {
        let req = json!({
            "jsonrpc": "2.0", "id": 4,
            "method": "tools/call",
            "params": { "name": "worktree_list", "arguments": {} }
        });
        let resp = dispatch(&req);
        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .expect("content text");
        assert!(text.contains("Phase 3"), "stub should mention Phase 3");
        assert_eq!(resp["result"]["isError"], true);
    }

    #[test]
    fn shutdown_returns_null_result() {
        let req = json!({ "jsonrpc": "2.0", "id": 5, "method": "shutdown" });
        let resp = dispatch(&req);
        assert_eq!(resp["result"], Value::Null);
        assert!(resp.get("error").is_none());
    }

    #[test]
    fn unknown_method_returns_method_not_found_code() {
        let req = json!({ "jsonrpc": "2.0", "id": 6, "method": "unknown/method" });
        let resp = dispatch(&req);
        assert_eq!(resp["error"]["code"], code::METHOD_NOT_FOUND);
    }

    #[test]
    fn malformed_json_handled_by_dispatcher_gracefully() {
        // dispatch() itself receives a valid Value; the parse-error path is
        // exercised in run() before dispatch. Test the missing-method path.
        let req = json!({ "jsonrpc": "2.0", "id": 7 });
        let resp = dispatch(&req);
        assert_eq!(resp["error"]["code"], code::PARSE_ERROR);
    }
}
