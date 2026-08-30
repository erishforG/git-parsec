# git-parsec MCP Quickstart

**Milestone**: v1.0  
**Refs**: #293, #241  
**Updated**: 2026-08-30

Connect Claude Desktop, Cursor, or any MCP-capable AI client to your git
worktree workflow in under five minutes.

---

## Prerequisites

| Requirement | Check |
|---|---|
| `parsec` installed and on `$PATH` | `parsec --version` |
| `git` ≥ 2.36 | `git --version` |
| `gh` (GitHub CLI) authenticated | `gh auth status` |
| Claude Desktop **or** Cursor | latest version |
| GitHub Personal Access Token (PAT) | needed for PR/CI tools |

---

## 1. Register git-parsec with Your Client

The easiest path is the automated installer:

```sh
# Claude Desktop
parsec mcp install --client=claude-desktop

# Cursor
parsec mcp install --client=cursor

# Preview what would be written without touching disk
parsec mcp install --client=claude-desktop --dry-run
```

`parsec mcp install` merges the `mcpServers.git-parsec` entry into the
client's existing JSON config, creates a timestamped backup of any previous
file, and preserves all other MCP server entries.

After running the command, **restart the client** to pick up the new server.

### What the installer writes

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

Config file locations:

| Client | macOS | Linux | Windows |
|---|---|---|---|
| Claude Desktop | `~/Library/Application Support/Claude/claude_desktop_config.json` | `~/.config/Claude/claude_desktop_config.json` | `%APPDATA%\Claude\claude_desktop_config.json` |
| Cursor | `~/Library/Application Support/Cursor/User/globalStorage/cursor.mcp.json` | `~/.config/Cursor/User/globalStorage/cursor.mcp.json` | `%APPDATA%\Cursor\User\globalStorage\cursor.mcp.json` |

### Manual setup (without the installer)

If you prefer to edit config by hand, add the block above to your client's
config file. To point at a local development build:

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

---

## 2. Configure GitHub Auth

Read-only tools (`health_check`, `smartlog`, `worktree_list`) work without a
token. Tools that call the GitHub API (`pr_status`, `ci_status`, `reviews`,
`worktree_ship`, …) need a delegated PAT.

### Option A — Config file (recommended)

Create `~/.config/parsec/mcp.toml`:

```toml
[auth]
token = "ghp_your_token_here"
# Declare only the scopes you want the AI agent to use.
# Omit 'scopes' to allow all available operations.
scopes = ["pull_request:read", "checks:read", "pull_request:write"]
```

| Scope | Grants |
|---|---|
| `pull_request:read` | `pr_status`, `reviews`, overlay in `smartlog` |
| `checks:read` | `ci_status` |
| `pull_request:write` | `worktree_ship` (push + open PR) |

The `PARSEC_MCP_CONFIG` environment variable overrides the default config path.

### Option B — Environment variable

Set `PARSEC_GITHUB_TOKEN` in your shell profile or client environment:

```sh
export PARSEC_GITHUB_TOKEN=ghp_your_token_here
```

The env var takes precedence over the config file when both are present.

> **Security note**: Never put `GITHUB_TOKEN` or `GH_TOKEN` in the MCP client
> JSON config — the installer refuses to do this. Use the mcp.toml file or
> set the env var in your shell profile.

---

## 3. Verify the Connection

Open Claude Desktop (or Cursor) and ask:

> *"List my git worktrees"*

You should see a response like:

```
You have 3 worktrees:
• main (checked out at /Users/you/project)
• feat/ABC-123 — locked, 2 uncommitted files
• fix/DEF-456 — clean, 1 commit ahead of develop
```

If you get a "server not found" error, check that:
1. You restarted the client after running `parsec mcp install`.
2. `parsec` is on `$PATH` in the client's environment (on macOS, GUI apps
   may inherit a different `$PATH` — use an absolute binary path if needed).
3. `parsec mcp serve` works from your terminal: type a JSON-RPC line and
   check the response.

---

## 4. Common AI Agent Workflows

### Health check before starting work

> *"Run parsec health check for my worktrees"*

The `health_check` tool reports uncommitted changes, lock files, and worktrees
that are stale relative to their base branch — no GitHub token needed.

### Create a worktree for a new ticket

> *"Start a new worktree for ticket ABC-123"*

The agent runs `worktree_start` with `dry_run=true` first, shows you the
planned branch name and base branch, then asks for confirmation before creating
the worktree. Mutating tools **always** preview before acting.

### Check PR and CI status

> *"Show me the CI status for my open PRs"*

Requires `checks:read` scope. The `ci_status` tool calls `gh pr view
--json statusCheckRollup` for each worktree and returns a summary.

> *"Which PRs are waiting for my review?"*

Requires `pull_request:read` scope. Uses `gh pr list --search
review-requested:@me`.

### Ship a worktree as a pull request

> *"Ship the ABC-123 worktree as a draft PR"*

`worktree_ship` requires `pull_request:write` scope and runs a two-step
confirmation:

1. `dry_run=true` — previews the push target and `gh pr create` arguments.
2. `confirm=true` — pushes the branch and opens the PR.

You must explicitly confirm before any branch is pushed or PR is opened.

### Smart log across worktrees

> *"Show me the commit graph across all my worktrees"*

`smartlog` returns a DAG of branches, their relationship to the base branch,
and — if a GitHub token with `pull_request:read` is present — PR state and
review status as an overlay.

---

## 5. Full Tool Catalogue

| Tool | Needs auth? | Mutating? | Description |
|---|---|---|---|
| `worktree_list` | No | No | All managed worktrees + metadata |
| `worktree_start` | No | Yes* | Create a new worktree for a ticket |
| `worktree_status` | No | No | Status of one worktree |
| `worktree_ship` | `pull_request:write` | Yes* | Push branch + open PR |
| `health_check` | No | No | Lock/uncommitted/stale summary |
| `smartlog` | Optional `pull_request:read` | No | Commit DAG ± PR/review overlay |
| `pr_status` | `pull_request:read` | No | PR state and merge readiness |
| `ci_status` | `checks:read` | No | Workflow / check-run summary |
| `reviews` | `pull_request:read` | No | PRs assigned for review |
| `sync` | No | Yes* | Rebase/merge stale worktrees |

\* Mutating tools require `dry_run=true` preview first, then `confirm=true`.

Detailed input/output schemas and error codes are in `docs/mcp/spec.md`.
Auth scopes and config-file format are in `docs/mcp/auth.md`.
Client config paths and installer details are in `docs/mcp/clients.md`.

---

## 6. Advanced: Running the Server Manually

You can drive the server from the terminal with any JSON-RPC client, or pipe
JSON directly:

```sh
# Smoke test: initialize → list tools → shutdown
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"curl-test","version":"0.0.0"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
{"jsonrpc":"2.0","id":3,"method":"shutdown"}' | parsec mcp serve
```

The server reads newline-delimited JSON-RPC from stdin and writes responses to
stdout. All human-readable diagnostics go to stderr so stdout stays
protocol-clean for client parsing.

---

## 7. Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| Client can't find server | `parsec` not on GUI `$PATH` | Use absolute path in client config or add to `/etc/paths` (macOS) |
| `AUTH_REQUIRED` error | No GitHub token configured | Set `PARSEC_GITHUB_TOKEN` or add `mcp.toml` |
| `INSUFFICIENT_SCOPE` | Token missing required scope | Add the scope to `mcp.toml` `scopes` array |
| `CONFIRMATION_REQUIRED` | Mutating tool skipped `dry_run` | Ask the agent to "preview first" or "use dry_run=true" |
| `SANDBOX_VIOLATION` | Tool requested path outside session boundary | Pass a path inside the server's repo root |
| Empty output from `smartlog` | No commits in current branch | Ensure you're in a parsec-managed worktree |
| `worktree_ship` fails on push | No remote configured | Run `git remote add origin <url>` in the worktree |

For server diagnostics, run `parsec mcp serve` in a terminal and inspect
stderr output — it contains structured JSON-LD audit events and Rust tracing
logs. Token values are never written to stdout, stderr, or audit events.

---

## 8. Next Steps

- Read `docs/mcp/spec.md` for full tool schemas, error codes, and the tool
  dependency map.
- Read `docs/mcp/auth.md` for the complete auth pipeline, scope matrix, and
  audit event specification.
- Read `docs/mcp/clients.md` for the `parsec mcp install` hook and manual
  client config reference.
- Explore `tests/mcp/fixtures/stdio_smoke.jsonl` for replay fixtures that
  cover every tool path — useful for writing custom integrations.

---

*Maintained by the git-parsec team. Changes require review by @erishforG.*
