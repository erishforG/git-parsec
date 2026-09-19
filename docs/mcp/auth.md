# git-parsec MCP Auth and Sandbox Model

**Version**: 1.0  
**Date**: 2026-08-31  
**Milestone**: v1.0  
**Refs**: #294, #241

## Goal

The MCP server lets AI clients call parsec tools without giving the server a
long-lived ambient credential. Auth is delegated per client session, scopes are
checked before GitHub-backed tools run, and filesystem access stays anchored to
the selected repository.

All described behaviors are fully implemented as of Phase 38. This document
reflects the shipped implementation and serves as the authoritative reference
for `src/mcp/` behavior.

## Credential Flow

MCP clients pass credentials into the server context, not through global process
state. Tool handlers use `McpContext.github_token` and do not read
`GITHUB_TOKEN`, `GH_TOKEN`, or credential helper state directly.

```text
MCP client
  -> initialize
  -> tools/list
  -> tools/call(name, arguments, delegated token metadata)
      -> McpContext { repo_path, github_token, github_scopes, dry_run }
      -> preflight validates required scope before network calls
```

The token is modeled as `Option<String>` in `McpContext`. Logs, errors, and
JSON responses never include the raw token. Fixtures in
`tests/mcp/fixtures/` use `"<redacted-token>"` placeholders, enforced by the
`stdio_recording_fixtures_are_redacted` unit test.

## Credential Sources

The server resolves a GitHub token from the following sources, in order:

1. **`PARSEC_GITHUB_TOKEN` env var** — if set and non-empty, used immediately;
   config file is skipped entirely. (Phase 35)
2. **`PARSEC_MCP_CONFIG` env var** — if set, overrides the config-file path.
   (Phase 36)
3. **`~/.config/parsec/mcp.toml`** — default path on Linux/macOS;
   `%APPDATA%\parsec\mcp.toml` on Windows. (Phase 36)

Config file format:

```toml
[auth]
token = "ghp_your_token_here"
# Optional: declare scopes so per-tool scope checks are enforced.
# When omitted, all scopes are treated as available (same as env-var delegation).
scopes = ["pull_request:read", "checks:read"]
```

Valid scope strings match the values in the Scope Matrix below:
`"pull_request:read"`, `"checks:read"`, `"pull_request:write"`.
Unknown scope strings are silently ignored so future scopes do not break
existing installations. The config file is read once at server startup;
dynamic reload is not supported.

## Scope Matrix

| Tool | GitHub token | Minimum scope |
|---|---|---|
| `worktree_list` | Optional | none unless `include_pr` or `include_ci` is true |
| `worktree_start` | Not required | none |
| `worktree_status` | Optional | read pull request metadata when requested |
| `worktree_ship` | Required | `pull_request:write` (push branch + create/update PR) |
| `smartlog` | Optional | `pull_request:read` + `checks:read` for overlays |
| `ci_status` | Required | `checks:read` |
| `pr_status` | Required | `pull_request:read` |
| `health_check` | Optional | `checks:read` when `include_ci` is true |
| `reviews` | Required | `pull_request:read` |
| `sync` | Not required | none |

`pull_request:write` implies `pull_request:read` for scope-gate purposes;
a write-scoped token passes all read-only preflight checks.

The server returns a structured MCP tool error before making a network
call when a required token is absent or the requested operation exceeds the
declared scope.

## Sandbox Rules

1. Resolve `repo` against the server current working directory when it is
   relative, then canonicalize it before use.
2. Reject `repo` paths that are not inside a git repository root recognized by
   parsec (`SANDBOX_VIOLATION`). Enforced in `preflight_tool_call`.
3. Ticket IDs are validated against an allowlist: no path traversal (`..`, `/`),
   no shell meta-characters, no null bytes, no whitespace.
4. Worktree discovery and mutation stay within the selected repository and
   parsec-managed sibling directories.
5. `dry_run` is honoured before any git mutation, network write, or
   filesystem-delete operation. Mutating tools require `confirm: true` to
   proceed (`CONFIRMATION_REQUIRED` when omitted).
6. Hook execution, template includes, and credential helpers are not exposed
   through MCP tool arguments.

## Threat Model

| Threat | Risk | Mitigation |
|---|---|---|
| Token leakage through stdout | High | stdout is JSON-RPC only; redact token-bearing errors |
| Ambient credential use | High | handlers read only from `McpContext` |
| Repository escape via `repo` | High | canonicalize + boundary check before dispatch |
| Accidental mutation by agent | Medium | `dry_run` + `CONFIRMATION_REQUIRED` preflight |
| Confused deputy across repositories | Medium | each tool call is bound to one resolved repo path |
| Secret persistence in fixtures | Medium | redaction contract in `tests/mcp/README.md` |
| Malicious hook or template path | Medium | hook/template execution not exposed via MCP |
| Audit-log injection via correlationId | Low | correlation IDs are allowlisted to safe characters |

## Error Contract

Auth and sandbox failures use the MCP `tools/call` content envelope with
`isError: true`:

```json
{
  "error": {
    "code": "AUTH_REQUIRED",
    "message": "Tool 'pr_status' requires a delegated GitHub token.",
    "detail": "Pass a session token with scopes: pull_request:read."
  }
}
```

Error codes:

| Code | Meaning |
|---|---|
| `AUTH_REQUIRED` | Tool requires a token and none was delegated |
| `INSUFFICIENT_SCOPE` | Token delegated but does not cover the requested operation |
| `SANDBOX_VIOLATION` | Requested path is outside the resolved repository boundary |
| `DRY_RUN_REQUIRED` | Server policy requires preview mode for a mutating call |
| `CONFIRMATION_REQUIRED` | Mutating tool called without `dry_run=true` or `confirm=true` |

## Audit Log

Every tool call emits one newline-delimited JSON event to stderr. Events
contain only: schema version, tool name, outcome, mutation classification,
dry-run flag, and an optional correlation ID. Arguments, paths, credentials,
and error messages are never included.

Outcomes: `allowed`, `denied`, `tool_error`.

Correlation IDs are copied from the JSON-RPC request ID when present. String
IDs must be 1–64 ASCII letters, digits, hyphens, underscores, or dots; other
strings and null IDs are omitted. The v1 schema is pinned in
`tests/mcp/fixtures/audit_event_v1.json`.

Audit emission is best-effort: sink failures do not interrupt tool calls or
change JSON-RPC responses.

## Implementation Phase History

| Phase | Work | Status |
|---|---|---|
| Phase 1 | Auth delegation, scope matrix, sandbox rules, threat model | ✅ shipped |
| Phase 2 | Redacted credential helpers, scope metadata in `src/mcp/` | ✅ shipped |
| Phase 3 | `AUTH_REQUIRED` preflight before GitHub-backed tool dispatch | ✅ shipped |
| Phase 4 | Fixture scrubbing rules for MCP e2e recordings | ✅ shipped |
| Phase 16 | `CONFIRMATION_REQUIRED` for non-preview mutating calls | ✅ shipped |
| Phase 17 | Privacy-safe structured audit events to stderr | ✅ shipped |
| Phase 18 | Optional privacy-safe correlation IDs for audit tracing | ✅ shipped |
| Phase 19 | Audit sink rotation, retention, and failure behavior | ✅ shipped |
| Phase 20 | v1 audit-event schema + compatibility fixture | ✅ shipped |
| Phase 21 | Testable newline-delimited audit writer | ✅ shipped |
| Phase 22 | Best-effort sink: failures do not interrupt tool calls | ✅ shipped |
| Phase 35 | `PARSEC_GITHUB_TOKEN` env var → `McpContext.github_token` | ✅ shipped |
| Phase 36 | `~/.config/parsec/mcp.toml` config-file auth (token + scopes) | ✅ shipped |
| Phase 37 | Config-file scope enforcement integration tests | ✅ shipped |
| Phase 38 | `mcp.toml` auth source + env-var precedence e2e; token-redaction assertions | ✅ shipped |

*Maintained by the git-parsec team. Auth changes require review by @erishforG.*
