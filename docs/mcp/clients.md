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

After registration, clients should be able to run `initialize` and `tools/list`.
The expected server identity is:

```json
{
  "name": "git-parsec"
}
```

The tool list should include `worktree_list`, `worktree_status`, `smartlog`,
`ci_status`, `pr_status`, `health_check`, `reviews`, and `sync`.

## Troubleshooting

| Symptom | Check |
|---|---|
| Client cannot start server | Use an absolute binary path and confirm `parsec mcp serve` runs in a shell |
| JSON parse failures | Confirm no logs or prompts are written to stdout |
| Tools cannot find repository | Pass an absolute `repo` argument in the tool call |
| GitHub-backed tools fail auth | Confirm the client delegated a token to MCP context |

## Next Phases

| Phase | Work |
|---|---|
| Phase 4 | Add an automated smoke fixture for client-style `initialize` and `tools/list` |
| Phase 5 | Document installer hooks once client config paths are finalized |

*Maintained by the git-parsec team. Client registration changes require review by @erishforG.*
