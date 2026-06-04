# git-parsec MCP Tool Specification

**Version**: 0.1 (draft)  
**Date**: 2026-06-04  
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

**Standard error codes**:

| Code | Meaning |
|---|---|
| `WORKTREE_NOT_FOUND` | Ticket has no managed worktree |
| `GIT_ERROR` | Underlying git2 / git CLI error |
| `GITHUB_API_ERROR` | GitHub API call failed (rate limit, auth, network) |
| `TRACKER_ERROR` | Jira / GitLab API error |
| `DIRTY_WORKTREE` | Operation blocked by uncommitted changes |
| `CONFLICT` | Merge/rebase conflict detected |
| `DRY_RUN` | Preview only; no changes made |

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
| Phase 1 (this PR) | `docs/mcp/spec.md` — tool catalogue + schemas |
| Phase 2 (#293) | `parsec mcp serve` skeleton — stdio JSON-RPC echo server |
| Phase 3 (#293) | Wire `worktree_list` + `worktree_status` to real impl |
| Phase 4 (#293) | Wire remaining tools; `parsec mcp serve` fully functional |
| Phase 5 (#294) | Auth — PAT delegation + scope checking |
| Phase 6 (#295) | e2e fixtures + Claude Desktop / Cursor integration tests |

---

*Maintained by the git-parsec team. Spec changes require review by @erishforG.*
