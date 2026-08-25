# git-parsec MCP Client Registration

**Version**: 0.1 (draft)  
**Date**: 2026-06-13  
**Milestone**: v1.0  
**Refs**: #293, #241

## Goal

This document fixes the Phase 3 registration contract for running
`parsec mcp serve` from desktop MCP clients. It keeps registration details in
`docs/mcp/` while the Rust transport continues to evolve in `src/mcp/`.

## Server Command

All clients should launch the server with stdio transport:

```sh
parsec mcp serve
```

The process reads newline-delimited JSON-RPC 2.0 from stdin and writes only
JSON-RPC responses to stdout. Human diagnostics must go to stderr so client
recordings stay protocol-clean.

When testing a local checkout, use an absolute path to the debug binary:

```sh
/path/to/git-parsec/target/debug/parsec mcp serve
```

## Claude Desktop

Add a server entry named `git-parsec` to the Claude Desktop MCP configuration:

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

## Cursor

Cursor should register the same stdio command shape:

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

If Cursor is opened outside the target repository, tool calls should pass the
`repo` argument explicitly so the server can resolve the intended git root.

## Client Config Files

Manual setup and future installer hooks should target only documented MCP
configuration files. If a path is missing, print the matching JSON block from
this document and let the user create the file.

| Client | Platform | Config file |
|---|---|---|
| Claude Desktop | macOS | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| Claude Desktop | Windows | `%APPDATA%\Claude\claude_desktop_config.json` |
| Cursor | macOS/Linux | `~/.cursor/mcp.json` |
| Cursor | Windows | `%USERPROFILE%\.cursor\mcp.json` |

Installer hooks must not guess alternate locations. When client vendors change
their config path, update this table before changing any automation.

### Merge Rules

When updating an existing config file, automation must:

- Parse the file as JSON before writing.
- Preserve unrelated top-level keys and unrelated `mcpServers` entries.
- Replace only the `mcpServers.git-parsec` entry.
- Keep `command` and `args` as separate fields; do not shell-join them.
- Stop without writing if the file contains comments, trailing commas, or other
  non-JSON syntax.

## Environment

The server must not depend on ambient GitHub credentials. GitHub-backed tools
should receive delegated credentials from the client session as described in
`docs/mcp/auth.md`.

Allowed process environment:

| Variable | Required | Notes |
|---|---|---|
| `PATH` | Yes | Must locate `parsec` unless `command` is absolute |
| `RUST_LOG` | No | Diagnostics only; never write protocol logs to stdout |

Disallowed as auth inputs:

| Variable | Reason |
|---|---|
| `GITHUB_TOKEN` | Ambient credential; use delegated MCP context |
| `GH_TOKEN` | Ambient credential; use delegated MCP context |

## Smoke Test

After registration, clients should be able to run `initialize`, `ping`,
`tools/list`, and `shutdown`. The expected server identity is:

```json
{
  "name": "git-parsec"
}
```

The tool list should include `worktree_list`, `worktree_status`, `smartlog`,
`ci_status`, `pr_status`, `health_check`, `reviews`, and `sync`.

The expected `ping` result is an empty JSON object:

```json
{}
```

The expected `shutdown` result is JSON `null`. Clients may then close stdin
without expecting additional stdout frames.

## Troubleshooting

| Symptom | Check |
|---|---|
| Client cannot start server | Use an absolute binary path and confirm `parsec mcp serve` runs in a shell |
| JSON parse failures | Confirm no logs or prompts are written to stdout |
| Tools cannot find repository | Pass an absolute `repo` argument in the tool call |
| GitHub-backed tools fail auth | Confirm the client delegated a token to MCP context |

## Installer Hook Contract

Future installer hooks may automate the JSON snippets above, but they must keep
manual registration as the source of truth. The hook should:

- Resolve the `parsec` binary path before writing client config.
- Detect only the Claude Desktop and Cursor config files listed above.
- Create a timestamped backup before modifying an existing config file.
- Preserve unrelated `mcpServers` entries and update only `git-parsec`.
- Support `--dry-run` output that prints the target file and JSON diff without
  writing to disk.
- Refuse to embed `GITHUB_TOKEN`, `GH_TOKEN`, or other credentials in config.

If a client config file cannot be parsed, the hook must stop and print the
manual JSON block from this document instead of rewriting the file.

## Typical AI Agent Workflow

The table below shows the sequence of MCP calls an AI client (Claude Desktop,
Cursor, etc.) makes when creating and shipping a feature branch.
Each step matches a fixture in `tests/mcp/fixtures/stdio_smoke.jsonl`
(names prefixed `scenario-e2e-`).

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

Replay fixtures for steps 1–5 and 7 are deterministic without a real
repository and are exercised by `cargo test --quiet`. Steps 6 and 8 require a
live git repository and GitHub credentials; they are covered by manual e2e
recordings in `tests/mcp/fixtures/stdio_smoke.jsonl` (prefix
`scenario-e2e-`) and the `#295` tracking issue.

## Next Phases

| Phase | Work |
|---|---|
| Phase 4–11 | Automated smoke fixtures, redaction checks, lifecycle fixtures (shipped) |
| Phase 32 | Claude Desktop / Cursor e2e scenario fixtures — `scenario-e2e-*` (this PR) |
| Phase 33 | Automated installer hook (`parsec mcp install --client=claude-desktop`) |
| Phase 34 | Live e2e recording with a sandboxed test repository (issue #295) |

*Maintained by the git-parsec team. Client registration changes require review by @erishforG.*
