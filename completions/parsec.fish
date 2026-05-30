# fish completion for git-parsec — dynamic worktree/branch candidates.
#
# Issue #291 Phase 2. Install:
#
#   cp completions/parsec.fish ~/.config/fish/completions/
#
# Companion to the static structure emitted by `parsec config completions fish`;
# this file replaces it with dynamic completion calling
# `parsec __complete worktrees|branches` for ticket and branch arguments.

# -- Dynamic candidate providers ----------------------------------------------
function __parsec_worktrees
    parsec __complete worktrees 2>/dev/null
end

function __parsec_branches
    parsec __complete branches 2>/dev/null
end

# -- Top-level subcommand list ------------------------------------------------
complete -c parsec -f -n __fish_use_subcommand -a start       -d 'Create new worktree'
complete -c parsec -f -n __fish_use_subcommand -a switch      -d 'Print worktree path'
complete -c parsec -f -n __fish_use_subcommand -a ship        -d 'Open or update PR'
complete -c parsec -f -n __fish_use_subcommand -a open        -d 'Open ticket/PR in browser'
complete -c parsec -f -n __fish_use_subcommand -a list        -d 'List all worktrees'
complete -c parsec -f -n __fish_use_subcommand -a status      -d 'Show ticket status'
complete -c parsec -f -n __fish_use_subcommand -a ticket      -d 'Show current ticket'
complete -c parsec -f -n __fish_use_subcommand -a clean       -d 'Remove merged worktree'
complete -c parsec -f -n __fish_use_subcommand -a pr-status   -d 'Show PR status'
complete -c parsec -f -n __fish_use_subcommand -a ci          -d 'Show CI status'
complete -c parsec -f -n __fish_use_subcommand -a merge       -d 'Merge PRs'
complete -c parsec -f -n __fish_use_subcommand -a diff        -d 'Diff against base'
complete -c parsec -f -n __fish_use_subcommand -a sync        -d 'Rebase/merge from base'
complete -c parsec -f -n __fish_use_subcommand -a log         -d 'Audit log'
complete -c parsec -f -n __fish_use_subcommand -a compress    -d 'Squash worktree commits'
complete -c parsec -f -n __fish_use_subcommand -a rename      -d 'Rename a ticket'
complete -c parsec -f -n __fish_use_subcommand -a adopt       -d 'Adopt existing branch'
complete -c parsec -f -n __fish_use_subcommand -a smartlog    -d 'Visualize worktrees as DAG'
complete -c parsec -f -n __fish_use_subcommand -a sl          -d 'Alias of smartlog'
complete -c parsec -f -n __fish_use_subcommand -a config      -d 'Configuration'
complete -c parsec -f -n __fish_use_subcommand -a doctor      -d 'Diagnose environment'
complete -c parsec -f -n __fish_use_subcommand -a health      -d 'Check worktree health'
complete -c parsec -f -n __fish_use_subcommand -a conflicts   -d 'Detect file overlap'
complete -c parsec -f -n __fish_use_subcommand -a history     -d 'Command history'
complete -c parsec -f -n __fish_use_subcommand -a stack       -d 'Stack-aware operations'
complete -c parsec -f -n __fish_use_subcommand -a release     -d 'Cut a release'

# -- Per-subcommand positional: worktree ticket -------------------------------
set -l ticket_cmds start switch ship open clean status ticket pr-status \
    ci merge diff sync log compress adopt rename

for cmd in $ticket_cmds
    complete -c parsec -f -n "__fish_seen_subcommand_from $cmd" \
        -a '(__parsec_worktrees)'
end

# -- Per-subcommand option flags ----------------------------------------------
# start: --base / --on / --branch / --title
complete -c parsec -f -n '__fish_seen_subcommand_from start' \
    -l base   -d 'Base branch' -a '(__parsec_branches)'
complete -c parsec -f -n '__fish_seen_subcommand_from start' \
    -l on     -d 'Stack on ticket' -a '(__parsec_worktrees)'
complete -c parsec -f -n '__fish_seen_subcommand_from start' \
    -l branch -d 'Use existing branch' -a '(__parsec_branches)'
complete -c parsec -n '__fish_seen_subcommand_from start' \
    -l title  -d 'Title for PR'

# ship: --base
complete -c parsec -f -n '__fish_seen_subcommand_from ship' \
    -l base   -d 'Base branch for PR' -a '(__parsec_branches)'

# smartlog: --depth / --no-overlay
complete -c parsec -n '__fish_seen_subcommand_from smartlog sl' \
    -l depth -d 'Max commits per worktree'
complete -c parsec -f -n '__fish_seen_subcommand_from smartlog sl' \
    -l no-overlay -d 'Skip GitHub PR/CI overlay'

# adopt: --branch
complete -c parsec -f -n '__fish_seen_subcommand_from adopt' \
    -l branch -d 'Branch to adopt' -a '(__parsec_branches)'

# -- Global flags --------------------------------------------------------------
complete -c parsec -f -l json  -d 'Emit JSON output'
complete -c parsec -f -l quiet -d 'Suppress output'
