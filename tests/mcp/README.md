# MCP Recording Fixtures

This directory stores MCP stdio recordings used by integration and smoke
tests. Fixtures are JSON Lines files so future Claude Desktop or Cursor
captures can append one request/response expectation per line.

Each non-empty, non-comment line has this shape:

```json
{"name":"initialize","request":{"jsonrpc":"2.0","id":1,"method":"initialize"},"assertions":[{"pointer":"/result/serverInfo/name","equals":"git-parsec"}]}
```

## Fields

- `name`: stable fixture label for failure messages.
- `request`: JSON-RPC request sent to `parsec mcp serve`.
- `assertions`: checks applied to the dispatcher response.

Supported assertion keys:

- `pointer`: required JSON Pointer into the response.
- `equals`: exact JSON value match.
- `kind`: JSON type check (`object`, `array`, `string`, `number`, `boolean`, or `null`).
- `min_len`: minimum array length.
- `contains_tool`: checks an array of MCP tool objects for a matching `name`.

Keep fixtures deterministic. Do not record machine-local paths, auth tokens,
timestamps, or network-derived fields.
