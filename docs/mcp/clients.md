# git-parsec MCP Client Registration

**Version**: 1.0  
**Date**: 2026-08-31  
**Milestone**: v1.0  
**Refs**: #293, #241

## Goal

This document is the reference for connecting desktop AI clients
(Claude Desktop, Cursor) to the `parsec mcp serve` stdio transport.
The automated installer (`parsec mcp install`) is the recommended path;
manual JSON snippets are provided as a fallback.

All described behaviors are fully implemented as of Phase 39. See
`docs/mcp-quickstart.md` for the end-to-end user workflow.

## Server Command

All clients launch the server with stdio transport:

```sh
parsec mcp serve
```

The process reads newline-delimited JSON-RPC 2.0 from stdin and writes only
JSON-RPC responses to stdout. Human diagnostics go to stderr so client
recordings stay protocol-clean.

When testing a local checkout, use an absolute path to the debug binary:

```sh
/path/to/git-parsec/target/debug/parsec mcp serve
```

## Automated Installer (Recommended)

Use `parsec mcp install` to write or update client config automatically:

```sh
# Preview without writing anything
parsec mcp install --client=claude-desktop --dry-run

# Write Claude Desktop config
parsec mcp install --client=claude-desktop

# Write Cursor config
parsec mcp install --client=cursor
```

The installer:
- Merges `mcpServers.git-parsec` into the existing client JSON config.
- Creates a timestamped backup of any previous file before overwriting.
- Preserves all other `mcpServers` entries and top-level keys.
- Refuses to write if the file contains non-JSON syntax (comments, trailing commas).
- Never embeds GitHub tokens or other credentials in client config.

After running the command, **restart the client** to pick up the new server.

## Manual Setup

If you prefer to configure clients by hand, use the snippets below.

### Claude Desktop

Add a `git-parsec` entry to the Claude Desktop MCP configuration:

```json
{
  "mcpServers": {
    "git-parsec": {
      "command": "parsec",
      "args": ["mcp", "serve"]
    }
  }
}
```

For local development, replace `command` with an absolute binary path:

```json
{
  "mcpServers": {
    "git-parsec-dev": {
      "command": "/path/to/git-parsec/target/debug/parsec",
      "args": ["mcp", "serve"]
    }
  }
}
```

### Cursor

Cursor uses the same stdio command shape:

```json
{
  "mcpServers": {
    "git-parsec": {
      "command": "parsec",
      "args": ["mcp", "serve"]
    }
  }
}
```

If Cursor is opened outside the target repository, pass the `repo` argument
explicitly in tool calls so the server can resolve the intended git root.

## Client Config Files

| Client | Platform | Config file |
|---|---|---|
| Claude Desktop | macOS | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| Claude Desktop | Windows | `%APPDATA%\Claude\claude_desktop_config.json` |
| Cursor | macOS/Linux | `~/.cursor/mcp.json` |
| Cursor | Windows | `%USERPROFILE%\.cursor\mcp.json` |

The installer resolves these paths automatically. Manual setup should use
the same paths; if a client vendor changes its config location, update this
table before changing any automation.

## Environment

The server does not depend on ambient GitHub credentials. GitHub-backed tools
receive delegated credentials from the client session as described in
`docs/mcp/auth.md`.

Allowed process environment:

| Variable | Required | Notes |
|---|---|---|
| `PATH` | Yes | Must locate `parsec` unless `command` is absolute |
| `PARSEC_GITHUB_TOKEN` | No | Delegated GitHub token for non-interactive hosts |
| `PARSEC_MCP_CONFIG` | No | Override path for `mcp.toml` config file |
| `RUST_LOG` | No | Diagnostics only; never write protocol logs to stdout |

Disallowed as auth inputs (ambient credentials — use delegated MCP context):

| Variable | Reason |
|---|---|
| `GITHUB_TOKEN` | Ambient credential; use `PARSEC_GITHUB_TOKEN` instead |
| `GH_TOKEN` | Ambient credential; use `PARSEC_GITHUB_TOKEN` instead |

## Smoke Test

After registration, clients should be able to run `initialize`, `ping`,
`tools/list`, and `shutdown`. The expected server identity is:

```json
{
  "name": "git-parsec"
}
```

The tool list must include at least 10 tools: `worktree_list`,
`worktree_start`, `worktree_status`, `worktree_ship`, `smartlog`,
`ci_status`, `pr_status`, `health_check`, `reviews`, and `sync`.

The expected `ping` result is an empty JSON object:

```json
{}
```

The expected `shutdown` result is JSON `null`. Clients may then close stdin
without expecting additional stdout frames.

Fixture-driven smoke tests run automatically via `cargo test`.

## Troubleshooting

| Symptom | Check |
|---|---|
| Client cannot start server | Use an absolute binary path; confirm `parsec mcp serve` runs in a shell |
| JSON parse failures | Confirm no logs or prompts are written to stdout |
| Tools cannot find repository | Pass an absolute `repo` argument in the tool call |
| GitHub-backed tools fail auth | Set `PARSEC_GITHUB_TOKEN` or configure `~/.config/parsec/mcp.toml` |

## Typical AI Agent Workflow

The table below shows the sequence of MCP calls an AI client makes when
creating and shipping a feature branch. Each step matches a fixture in
`tests/mcp/fixtures/stdio_smoke.jsonl` (names prefixed `scenario-e2e-`).

| Step | Method / Tool | Key arguments | Expected outcome |
|---|---|---|---|
| 1 | `initialize` | `protocolVersion: "2025-03-26"` | Server info + capability advertisement |
| 2 | `tools/list` | — | Catalogue of ≥10 parsec tools |
| 3 | `health_check` | — | Lock/uncommitted/stale summary |
| 4 | `worktree_list` | — | Count + per-worktree metadata |
| 5 | `worktree_start` | `ticket`, `dry_run: true` | Preview: branch name + base branch |
| 6 | `worktree_start` | `ticket`, `confirm: true` | Worktree created at `path` |
| 7 | `worktree_ship` | `ticket`, `dry_run: true`, auth token | Preview: push + PR create plan |
| 8 | `worktree_ship` | `ticket`, `confirm: true`, auth token | Branch pushed, PR created |

Steps 5 and 7 should always run first (`dry_run: true`) so the AI client can
explain the planned change to the user before committing side effects.
Steps 6 and 8 require explicit user confirmation and a delegated GitHub token
with the `pull_request:write` scope.

Deterministic fixtures for steps 1–5 and 7 are exercised by `cargo test`.
Steps 6 and 8 require a live repository and are covered by recordings in
`tests/mcp/fixtures/stdio_smoke.jsonl` (prefix `scenario-e2e-`).

## Implementation Phase History

| Phase | Work | Status |
|---|---|---|
| Phase 3 | Registration contract, JSON config snippets, smoke-test expectations | ✅ shipped |
| Phases 4–11 | Automated smoke fixtures, redaction checks, lifecycle fixtures | ✅ shipped |
| Phase 32 (#295) | Claude Desktop / Cursor e2e scenario fixtures (`scenario-e2e-*`) | ✅ shipped |
| Phase 33 (#293) | Automated installer (`parsec mcp install --client=claude-desktop/cursor`) | ✅ shipped |
| Phase 34 (#295) | Live subprocess e2e with sandboxed test repository (10 tests) | ✅ shipped |
| Phase 39 (#293) | End-to-end user quickstart (`docs/mcp-quickstart.md`) | ✅ shipped |
| Phase 41 (#293) | Docs finalised to v1.0 — stale draft markers and "future phases" removed | ✅ shipped |

*Maintained by the git-parsec team. Client registration changes require review by @erishforG.*
