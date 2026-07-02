# git-parsec MCP Tool Specification

**Version**: 0.1 (draft)  
**Date**: 2026-07-02  
**Milestone**: v1.0  
**Refs**: #292, #241

## Overview

git-parsec exposes its worktree-native git workflow as an
[MCP (Model Context Protocol)](https://modelcontextprotocol.io/) server,
allowing AI agents (Claude Desktop, Cursor, etc.) to create, inspect, and
ship code across isolated git worktrees.

The transport is **stdio JSON-RPC 2.0** (`parsec mcp serve`). All tools
follow the MCP `tools/call` schema with `name`, `description`, and
`inputSchema` (JSON Schema draft-07).

---

## Protocol Contract

Phase 2 fixes the server boundary that later implementation phases must
preserve. The server speaks newline-delimited JSON-RPC 2.0 over stdio and
does not emit progress logs on stdout. Human-readable diagnostics go to
stderr so MCP clients can treat stdout as a pure protocol stream.

### Methods

| Method | Direction | Purpose |
|---|---|---|
| `initialize` | client -> server | Negotiate protocol version and server capabilities |
| `tools/list` | client -> server | Return the catalogue in this document |
| `tools/call` | client -> server | Invoke one registered parsec tool |

`tools/list` returns the registry from `src/mcp/mod.rs` and each tool's
`inputSchema`. `tools/call` accepts:

```json
{
  "name": "worktree_status",
  "arguments": {
    "ticket": "PROJ-123",
    "repo": "/repo"
  }
}
```

### Shared Inputs

All tools accept these common optional fields unless the individual tool
schema narrows them:

| Field | Type | Default | Notes |
|---|---|---|---|
| `repo` | string | current working directory | Absolute paths are preferred; relative paths resolve against server CWD |
| `dry_run` | boolean | tool-specific | Mutating tools must preview side effects when true |

Tools that call GitHub APIs require a delegated token from the MCP caller
context. They must not read `GITHUB_TOKEN` or `GH_TOKEN` directly.

### Delegated Auth Metadata

Phase 8 clarifies how MCP clients describe delegated GitHub auth without
placing secrets in the normal tool arguments. Clients may attach auth metadata
to the session context during initialization or to an out-of-band host channel,
but `tools/call.params.arguments` must remain limited to tool inputs. Tool
handlers receive the resolved metadata through `McpContext`, not by parsing
tool-specific JSON.

The server capability metadata should advertise supported scope identifiers so
clients can decide whether a token is useful before a tool call:

```json
{
  "capabilities": {
    "tools": {},
    "gitParsec": {
      "githubScopes": [
        "pull_request:read",
        "checks:read",
        "pull_request:write"
      ]
    }
  }
}
```

When a token is delegated, the client should provide the token value only to
the MCP host credential boundary. The parsec server receives a redacted
context equivalent to:

```json
{
  "github": {
    "token": "<delegated-secret>",
    "scopes": ["pull_request:read", "checks:read"]
  }
}
```

This object is illustrative and must not be serialized into checked-in
fixtures, stdout protocol responses, or tool result payloads.

### Scope Negotiation

Each registered tool has three auth properties:

| Property | Meaning |
|---|---|
| `requires_github` | A token is mandatory before the tool can run normally |
| `github_scopes` | Minimum scope identifiers needed for GitHub-backed behavior |
| Optional overlays | Arguments such as `include_pr` and `include_ci` can activate additional scope checks |

The dispatcher should validate mandatory auth before calling a handler. Tools
with optional GitHub overlays may run without a token only when the caller has
left those overlays disabled or accepts a degraded local-only response. If the
caller requests an overlay that needs GitHub access and no suitable token was
delegated, return the standard MCP error envelope with `AUTH_REQUIRED` or
`INSUFFICIENT_SCOPE`.

For v1.0 the stable scope identifiers are:

| Scope | Covers |
|---|---|
| `pull_request:read` | PR state, reviews, mergeability, review requests |
| `checks:read` | GitHub Actions check runs and workflow conclusions |
| `pull_request:write` | Branch push, PR creation, PR update |

`pull_request:write` implies `pull_request:read` for parsec-owned PR
operations, but it does not imply `checks:read`.

### Shared Response Envelope

Successful `tools/call` responses use MCP content blocks with one JSON payload
block. The payload is the tool-specific response shown below:

```json
{
  "content": [
    {
      "type": "text",
      "text": "{\"ticket\":\"PROJ-123\",\"ahead\":3,\"behind\":0}"
    }
  ],
  "isError": false
}
```

Errors use the same envelope with `isError: true` and the standard error body
defined in the Error Schema section. Future clients should key off
`error.code`, not the English `message`.

### Mutating Tool Rules

Mutating tools are `worktree_start`, `worktree_ship`, and `sync`.

- `dry_run: true` must not create worktrees, push branches, open PRs, rebase,
  merge, or delete files.
- `dry_run: false` may perform side effects only after input validation and
  repository discovery succeed.
- If a mutation is blocked by dirty state or conflicts, return a structured
  error and leave recovery to the caller.

---

## Design Principles

1. **Worktree-native** — every tool operates on a named *worktree* (ticket
   identifier), never on bare git state.
2. **Safe defaults** — mutations require explicit confirmation or produce
   preview output by default.
3. **Composable** — tools return structured JSON; AI agents compose them.
4. **No breaking CLI change** — the MCP layer calls into the same internal
   Rust functions as the CLI; `src/mcp/` never edits `src/cli/`.
5. **PAT delegation** — auth tokens are passed via caller context, not
   stored inside the server process (see `docs/mcp/auth.md`).

---

## Tool Catalogue

### 1. `worktree_list`

Return all parsec-managed worktrees and their current state.

```json
{
  "name": "worktree_list",
  "description": "List all active parsec worktrees with ticket, branch, PR, and CI status.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "repo": {
        "type": "string",
        "description": "Absolute path to the git repository root. Defaults to CWD."
      },
      "include_pr": {
        "type": "boolean",
        "description": "Fetch GitHub PR status for each worktree (requires network).",
        "default": true
      },
      "include_ci": {
        "type": "boolean",
        "description": "Fetch CI check-run status for each worktree (requires network).",
        "default": false
      }
    },
    "required": []
  }
}
```

**Response** (success):
```json
{
  "worktrees": [
    {
      "ticket": "PROJ-123",
      "branch": "feat/proj-123-my-feature",
      "path": "/repo/.parsec/PROJ-123",
      "created_at": "2026-06-01T09:00:00Z",
      "behind_main": 2,
      "pr": { "number": 42, "state": "open", "review_state": "approved" },
      "ci": null
    }
  ]
}
```

---

### 2. `worktree_start`

Create a new git worktree for a ticket identifier.

```json
{
  "name": "worktree_start",
  "description": "Create an isolated git worktree for a ticket. Fetches ticket title from configured tracker (Jira/GitHub Issues) when available.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "ticket": {
        "type": "string",
        "description": "Ticket identifier, e.g. 'PROJ-123' or '#42'."
      },
      "repo": {
        "type": "string",
        "description": "Absolute path to the git repository root."
      },
      "base": {
        "type": "string",
        "description": "Base branch to create from. Defaults to repository main/master."
      },
      "title": {
        "type": "string",
        "description": "Manually override ticket title (skips tracker lookup)."
      },
      "on": {
        "type": "string",
        "description": "Stack on top of another ticket's branch."
      },
      "dry_run": {
        "type": "boolean",
        "description": "Preview what would happen without making changes.",
        "default": false
      }
    },
    "required": ["ticket"]
  }
}
```

**Response** (success):
```json
{
  "ticket": "PROJ-123",
  "branch": "feat/proj-123-my-feature",
  "path": "/repo/.parsec/PROJ-123",
  "base_commit": "abc1234",
  "dry_run": false
}
```

---

### 3. `worktree_status`

Return the current status of a single worktree (uncommitted files, ahead/behind, PR, CI).

```json
{
  "name": "worktree_status",
  "description": "Show detailed status of a worktree: uncommitted changes, ahead/behind base, PR link, CI checks.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "ticket": {
        "type": "string",
        "description": "Ticket identifier for the target worktree."
      },
      "repo": { "type": "string" }
    },
    "required": ["ticket"]
  }
}
```

**Response** (success):
```json
{
  "ticket": "PROJ-123",
  "branch": "feat/proj-123-my-feature",
  "ahead": 3,
  "behind": 0,
  "uncommitted": 2,
  "pr": { "number": 42, "url": "https://github.com/org/repo/pull/42", "state": "open" },
  "ci": { "overall": "success", "runs": 5, "failed": 0 }
}
```

---

### 4. `worktree_ship`

Push the worktree branch, open (or update) a GitHub PR, and optionally clean up.

```json
{
  "name": "worktree_ship",
  "description": "Push the worktree branch to origin, create/update a GitHub PR, and optionally remove the local worktree.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "ticket": {
        "type": "string",
        "description": "Ticket identifier for the worktree to ship."
      },
      "repo": { "type": "string" },
      "draft": {
        "type": "boolean",
        "description": "Open PR as draft.",
        "default": false
      },
      "no_cleanup": {
        "type": "boolean",
        "description": "Skip removing the local worktree after shipping.",
        "default": false
      },
      "dry_run": {
        "type": "boolean",
        "default": false
      }
    },
    "required": ["ticket"]
  }
}
```

**Response** (success):
```json
{
  "ticket": "PROJ-123",
  "pr_url": "https://github.com/org/repo/pull/42",
  "pr_number": 42,
  "worktree_removed": true,
  "dry_run": false
}
```

---

### 5. `smartlog`

Render the commit DAG with worktree branches, PR/CI overlays, and stack indicators.

```json
{
  "name": "smartlog",
  "description": "Render the smartlog DAG: commit graph annotated with worktree branches, PR state, CI status, and stack relationships.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "repo": { "type": "string" },
      "ticket": {
        "type": "string",
        "description": "Filter to a single worktree's branch history."
      },
      "limit": {
        "type": "integer",
        "description": "Maximum commits to include.",
        "default": 50,
        "minimum": 1,
        "maximum": 500
      },
      "no_color": {
        "type": "boolean",
        "description": "Disable ANSI color codes in output.",
        "default": true
      }
    },
    "required": []
  }
}
```

**Response** (success):
```json
{
  "text": "* abc1234 (feat/proj-123) [PR#42 ✓CI] My feature\n| * def5678 (feat/proj-456) [PR#43 ⏳CI] Another feature\n...",
  "worktree_count": 3
}
```

---

### 6. `ci_status`

Fetch CI check-run results for a worktree's branch.

```json
{
  "name": "ci_status",
  "description": "Fetch GitHub Actions CI check-run status for a worktree branch.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "ticket": { "type": "string" },
      "repo": { "type": "string" },
      "limit": {
        "type": "integer",
        "description": "Max check runs to return.",
        "default": 10
      }
    },
    "required": ["ticket"]
  }
}
```

**Response** (success):
```json
{
  "ticket": "PROJ-123",
  "branch": "feat/proj-123-my-feature",
  "overall": "success",
  "runs": [
    { "name": "CI / build", "status": "completed", "conclusion": "success", "url": "..." },
    { "name": "CI / test", "status": "completed", "conclusion": "success", "url": "..." }
  ]
}
```

---

### 7. `pr_status`

Get the current GitHub PR state, review status, and merge readiness for a worktree.

```json
{
  "name": "pr_status",
  "description": "Return GitHub PR state, review approvals, merge readiness, and review comments for a worktree.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "ticket": { "type": "string" },
      "repo": { "type": "string" }
    },
    "required": ["ticket"]
  }
}
```

**Response** (success):
```json
{
  "ticket": "PROJ-123",
  "pr_number": 42,
  "pr_url": "https://github.com/org/repo/pull/42",
  "state": "open",
  "draft": false,
  "mergeable": true,
  "review_state": "approved",
  "approvals": 2,
  "change_requests": 0,
  "ci_overall": "success"
}
```

---

### 8. `health_check`

Run parsec's health diagnostics across all (or one) worktree(s).

```json
{
  "name": "health_check",
  "description": "Run worktree health diagnostics: lock files, uncommitted changes, stale branches, CI failures.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "ticket": {
        "type": "string",
        "description": "Scope to a single worktree. Omit to check all."
      },
      "repo": { "type": "string" },
      "include_ci": {
        "type": "boolean",
        "default": false
      }
    },
    "required": []
  }
}
```

**Response** (success):
```json
{
  "healthy": false,
  "worktrees": [
    {
      "ticket": "PROJ-123",
      "issues": [
        { "kind": "stale_branch", "severity": "warning", "detail": "Branch is 14 commits behind main" }
      ]
    }
  ]
}
```

---

### 9. `reviews`

List open GitHub PRs that the caller has been requested to review, or their own PRs awaiting review.

```json
{
  "name": "reviews",
  "description": "List PRs where the authenticated user is a requested reviewer, or their own PRs awaiting approval.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "repo": { "type": "string" },
      "filter": {
        "type": "string",
        "enum": ["incoming", "outgoing", "all"],
        "description": "incoming = review requests; outgoing = own PRs; all = both.",
        "default": "all"
      },
      "limit": {
        "type": "integer",
        "default": 20
      }
    },
    "required": []
  }
}
```

**Response** (success):
```json
{
  "incoming": [
    { "pr_number": 55, "title": "feat: ...", "author": "alice", "url": "...", "review_state": "pending" }
  ],
  "outgoing": [
    { "pr_number": 42, "title": "feat/proj-123", "reviewer": "bob", "url": "...", "review_state": "approved" }
  ]
}
```

---

### 10. `sync`

Sync stale worktrees with the latest base branch commits.

```json
{
  "name": "sync",
  "description": "Rebase or merge-update stale worktrees against the current main/develop branch.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "ticket": {
        "type": "string",
        "description": "Target a specific worktree. Omit to sync all stale ones."
      },
      "repo": { "type": "string" },
      "strategy": {
        "type": "string",
        "enum": ["rebase", "merge"],
        "default": "rebase"
      },
      "dry_run": {
        "type": "boolean",
        "default": true,
        "description": "Default true — preview conflicts before committing."
      },
      "stale_days": {
        "type": "integer",
        "description": "Only sync worktrees behind by at least this many base commits.",
        "default": 5
      }
    },
    "required": []
  }
}
```

**Response** (success):
```json
{
  "synced": ["PROJ-123"],
  "skipped": [],
  "conflicts": [],
  "dry_run": true
}
```

---

## Error Schema

All tools return MCP `isError: true` with a structured body on failure:

```json
{
  "error": {
    "code": "WORKTREE_NOT_FOUND",
    "message": "No worktree found for ticket 'PROJ-999'",
    "detail": "Run worktree_list to see available tickets."
  }
}
```

Current Phase 3 transport stubs use the same shape with `tool_error` while a
registered tool is reachable through `tools/call` but not yet implemented:

```json
{
  "error": {
    "code": "tool_error",
    "message": "worktree_status: implementation is planned for a later MCP phase (#293)",
    "tool": "worktree_status"
  }
}
```

**Standard error codes**:

| Code | Meaning |
|---|---|
| `tool_error` | Registered tool handler failed or is still a structured stub |
| `WORKTREE_NOT_FOUND` | Ticket has no managed worktree |
| `GIT_ERROR` | Underlying git2 / git CLI error |
| `GITHUB_API_ERROR` | GitHub API call failed (rate limit, auth, network) |
| `TRACKER_ERROR` | Jira / GitLab API error |
| `DIRTY_WORKTREE` | Operation blocked by uncommitted changes |
| `CONFLICT` | Merge/rebase conflict detected |
| `DRY_RUN` | Preview only; no changes made |
| `AUTH_REQUIRED` | Tool requires a delegated GitHub token and none was provided |
| `INSUFFICIENT_SCOPE` | Delegated token does not cover the requested GitHub operation |
| `SANDBOX_VIOLATION` | Requested repository or path escapes the validated parsec boundary |
| `DRY_RUN_REQUIRED` | Server policy requires preview mode for the requested mutation |

---

## Tool Dependency Map

```
worktree_list       ──► (no deps)
worktree_start      ──► tracker (optional), git2
worktree_status     ──► git2, github_api (optional)
worktree_ship       ──► git2, github_api
smartlog            ──► git2, github_api (optional)
ci_status           ──► github_api
pr_status           ──► github_api
health_check        ──► git2, github_api (optional)
reviews             ──► github_api
sync                ──► git2
```

---

## Implementation Notes

- All MCP tools live in `src/mcp/tools/` as individual Rust modules.
- Shared JSON serialisation uses `serde_json`; input validation uses the
  existing `anyhow` error chain.
- Auth tokens are injected via `McpContext` (see `docs/mcp/auth.md`),
  not read from env inside tool handlers.
- `dry_run` is a first-class parameter on all mutating tools and must be
  honoured before any side-effecting call.
- Response structs derive `serde::Serialize`; schemas in this doc are
  generated from those structs (to be automated in Phase 2).

---

## Next Phases

| Phase | Work |
|---|---|
| Phase 1 | `docs/mcp/spec.md` — tool catalogue + schemas |
| Phase 2 (this PR) | Protocol contract — JSON-RPC methods, shared inputs, response envelope |
| Phase 3 (#293) | `parsec mcp serve` skeleton — stdio JSON-RPC echo server |
| Phase 4 (#293) | Wire `worktree_list` + `worktree_status` to real impl |
| Phase 5 (#293) | Wire remaining tools; `parsec mcp serve` fully functional |
| Phase 6 (#294) | Auth — PAT delegation, scope checking, and sandbox rules |
| Phase 7 (#295) | e2e fixtures + Claude Desktop / Cursor integration tests |

---

*Maintained by the git-parsec team. Spec changes require review by @erishforG.*
