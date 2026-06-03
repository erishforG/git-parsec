# Error message format (3-line standard)

Issue [#303](https://github.com/erishforG/git-parsec/issues/303). All
user-facing errors should follow this format so users can quickly
distinguish *what* failed, *why*, and *what to do next*:

```
error: <short summary> [<ErrorCode>]
caused by: <upstream cause in plain language, optional>
help: <what the user should do next, optional>
```

Lines 2 and 3 are optional. If you only have the summary, that's a single
line — the format is additive.

## Why this matters

The first user contact with a failure is almost always a CLI line. If the
line answers all three of *what / why / now what?* the user does not need
to leave the terminal. If only *what* is shown, the user has to grep code
or open docs to figure out the next step.

## How to write one

Build a `ParsecError` with the builder methods:

```rust
use crate::errors::{ErrorCode, ParsecError};

return Err(ParsecError::new(
    ErrorCode::E005,
    format!("workspace '{}' not found", ticket),
)
.with_caused_by(format!(
    "directory missing or .git/parsec/state.json out of sync ({})",
    path.display()
))
.with_help("run `parsec doctor` to diagnose, or `parsec clean --orphans` to drop stale state")
.into());
```

Renders as:

```
error: workspace 'CL-2283' not found [E005]
caused by: directory missing or .git/parsec/state.json out of sync (/Users/.../parsec/state.json)
help: run `parsec doctor` to diagnose, or `parsec clean --orphans` to drop stale state
```

JSON mode (`--json`) renders the same fields:

```json
{
  "error": true,
  "code": "E005",
  "message": "workspace 'CL-2283' not found",
  "caused_by": "directory missing or .git/parsec/state.json out of sync (/Users/.../parsec/state.json)",
  "help": "run `parsec doctor` to diagnose, or `parsec clean --orphans` to drop stale state"
}
```

`caused_by` and `help` use `skip_serializing_if = "Option::is_none"`, so
existing JSON consumers see no schema change for errors that don't yet
adopt the format.

## When to fill each line

| Line | Fill when… | Skip when… |
|---|---|---|
| `error:` (always) | Always required — short, no period at the end. Mention the user-facing identifier (ticket / branch / file). | — |
| `caused by:` | The actual upstream reason is non-obvious or contains a path / numeric / external code. | The summary already names the cause unambiguously. |
| `help:` | There is a concrete next command, config key, or doc link. | Truly unrecoverable — but those are rare; prefer at least naming the docs. |

## Quick recipes

- **Missing config / token** → `caused by` names the env var / config key
  searched, `help` lists the resolution order (e.g., `PARSEC_GITHUB_TOKEN`,
  `gh auth login`).
- **State drift** (`.git/parsec/state.json` out of sync with disk) →
  `caused by` mentions the path, `help` recommends `doctor` or
  `clean --orphans`.
- **Network / forge error** → `caused by` includes the HTTP status and
  the URL path (no secrets), `help` suggests `--offline` or retry.
- **Hook failure** → `caused by` includes the hook command and exit
  code, `help` links to the hook config doc.

## What not to do

- ❌ Don't put the full anyhow chain into `caused by` — that's what the
  underlying error chain is for. `caused by` should be one line a human
  reads first.
- ❌ Don't include secrets (tokens, passwords, paths under `~/.config/`
  that include credentials) — assume the line shows up in CI logs.
- ❌ Don't end any line with a period — match `git`'s house style.
- ❌ Don't write `help` as a question ("did you forget to set X?"). Make
  it imperative ("set X" or "run `parsec ...`").

## Migration

This PR adds the format; the existing `ParsecError::new(...)` call sites
keep rendering as a single line. Migrate them gradually:

1. Whenever you touch an error site for any reason, add `with_caused_by`
   and / or `with_help`.
2. Prioritize sites in `cli/commands/` and `worktree/` (highest user
   contact).
3. Untyped `anyhow::anyhow!(...)` errors in user-facing paths should be
   converted to `ParsecError::new(ErrorCode::E???, ...)` over time.

The `bail_code!` macro stays as a quick path for the common "summary
only" case. For richer errors, build the `ParsecError` directly.
