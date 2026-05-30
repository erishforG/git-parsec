# bash completion for git-parsec — dynamic worktree/branch candidates.
#
# Issue #291 Phase 2. Source this file from ~/.bashrc:
#
#   source /path/to/git-parsec/completions/parsec.bash
#
# Or install to the bash-completion directory:
#
#   cp completions/parsec.bash /etc/bash_completion.d/parsec
#
# Companion to the static structure emitted by `parsec config completions bash`;
# this file replaces it with dynamic completion calling
# `parsec __complete worktrees|branches` for ticket and branch arguments.

_parsec_subcommands="start switch ship open list status ticket clean pr-status \
ci merge diff sync log compress rename adopt smartlog sl config doctor health \
conflicts history stack release"

_parsec_worktrees() {
    parsec __complete worktrees 2>/dev/null
}

_parsec_branches() {
    parsec __complete branches 2>/dev/null
}

_parsec() {
    local cur prev words cword
    _init_completion || return

    # Find the subcommand (first non-flag word after `parsec`).
    local sub=""
    local i=1
    while [ $i -lt $cword ]; do
        case "${words[i]}" in
            --json|--quiet) ;;
            -*) ;;
            *) sub="${words[i]}"; break ;;
        esac
        ((i++))
    done

    # Completing the subcommand itself.
    if [ -z "$sub" ]; then
        COMPREPLY=( $(compgen -W "$_parsec_subcommands" -- "$cur") )
        return
    fi

    # Option arguments first (only --base / --on / --branch take dynamic values).
    case "$prev" in
        --base|--branch)
            COMPREPLY=( $(compgen -W "$(_parsec_branches)" -- "$cur") )
            return
            ;;
        --on)
            COMPREPLY=( $(compgen -W "$(_parsec_worktrees)" -- "$cur") )
            return
            ;;
        --depth|--title)
            return # free text
            ;;
    esac

    # Don't complete option flag values, defer to default.
    if [[ "$cur" == -* ]]; then
        case "$sub" in
            start)
                COMPREPLY=( $(compgen -W "--base --on --branch --title" -- "$cur") )
                ;;
            ship)
                COMPREPLY=( $(compgen -W "--base --reviewer" -- "$cur") )
                ;;
            smartlog|sl)
                COMPREPLY=( $(compgen -W "--depth --no-overlay --json" -- "$cur") )
                ;;
            *)
                COMPREPLY=( $(compgen -W "--json --quiet" -- "$cur") )
                ;;
        esac
        return
    fi

    # Positional argument by subcommand.
    case "$sub" in
        start|switch|ship|open|clean|status|ticket|pr-status|diff|sync|compress|log|adopt|rename)
            COMPREPLY=( $(compgen -W "$(_parsec_worktrees)" -- "$cur") )
            ;;
        merge|ci)
            COMPREPLY=( $(compgen -W "$(_parsec_worktrees)" -- "$cur") )
            ;;
        *)
            # Fall back to filename completion for unknown subcommands.
            _filedir
            ;;
    esac
}

complete -F _parsec parsec
