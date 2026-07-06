# MCP Recording Fixtures

This directory stores MCP stdio recordings used by integration and smoke
tests. Fixtures are JSON Lines files so future Claude Desktop or Cursor
captures can append one request/response expectation per line.

Each non-empty, non-comment line has this shape:

```json
{"name":"initialize","request":{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"fixture-client","version":"0.0.0"}}},"assertions":[{"pointer":"/result/serverInfo/name","equals":"git-parsec"}]}
{"name":"initialized-notification","request":{"jsonrpc":"2.0","method":"notifications/initialized"},"no_response":true}
{"name":"ping","request":{"jsonrpc":"2.0","id":"ping","method":"ping"},"assertions":[{"pointer":"/result","kind":"object"}]}
{"name":"tools-call-worktree-list","request":{"jsonrpc":"2.0","id":"worktrees","method":"tools/call","params":{"name":"worktree_list","arguments":{}}},"assertions":[{"pointer":"/result/isError","equals":false},{"pointer":"/result/content/0/text","contains_text":"\"worktrees\""}]}
```

## Fields

- `name`: stable fixture label for failure messages.
- `request`: JSON-RPC request sent to `parsec mcp serve`.
- `assertions`: checks applied to the dispatcher response.
- `no_response`: set to `true` for JSON-RPC notifications that must not emit
  a response.

Fixture names must be unique within a recording file. Response fixtures must
declare at least one assertion, while notification fixtures use `no_response`
without assertions.
Initialize fixtures should include deterministic client metadata so the stdio
contract matches desktop MCP client handshakes without recording local state.

Supported assertion keys:

- `pointer`: required JSON Pointer into the response.
- `equals`: exact JSON value match.
- `kind`: JSON type check (`object`, `array`, `string`, `number`, `boolean`, or `null`).
- `contains_text`: substring check for deterministic error or content strings.
- `min_len`: minimum array length.
- `contains_tool`: checks an array of MCP tool objects for a matching `name`.

Keep fixtures deterministic. Do not record machine-local paths, auth tokens,
timestamps, or network-derived fields.
For wired tool calls, assert stable envelope fields and schema keys instead of
repository-specific counts, paths, branch names, or timestamps.

## Redaction Contract

Recording fixtures are committed test inputs, so they must be safe to publish
and stable across machines. Before adding a new recording, replace volatile or
secret-bearing values with deterministic placeholders.

| Source value | Fixture placeholder |
|---|---|
| GitHub tokens, PATs, bearer tokens | `<redacted-token>` |
| `Authorization` headers | `<redacted-authorization>` |
| User home directories or absolute repo paths | `<repo>` |
| Temporary directories | `<tmp>` |
| Real user emails | `<user@example.invalid>` |
| Wall-clock timestamps | `<timestamp>` |
| Network request IDs | `<request-id>` |

Reviewers should reject fixtures that contain token-shaped strings such as
`ghp_`, `github_pat_`, `Bearer `, or `Authorization`. Fixture responses should
also avoid raw stdout/stderr dumps from MCP clients; assert only the stable JSON
fields needed by the smoke contract.

## Recording Checklist

1. Capture the smallest request/response pair that exercises the behavior.
2. Redact secrets, local paths, timestamps, and network-generated IDs.
3. Prefer `contains_text`, `kind`, `min_len`, and `contains_tool` over copying
   full response payloads.
4. Run `cargo test --quiet mcp::tests::stdio_recording_fixtures_match_dispatcher`
   before opening a PR.
5. Run `cargo test --quiet mcp::tests::stdio_recording_fixtures_are_redacted`
   after adding or updating any committed recording.
