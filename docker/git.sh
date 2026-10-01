#!/bin/bash
set -u

options=()
args=("$@")
while [[ ${#args[@]} -gt 0 ]]; do
    case ${args[0]} in
        --version|-v|--help|-h|--exec-path|--exec-path=*)
            exec /usr/bin/git "$@"
            ;;
        -C|-c|--git-dir|--work-tree|--namespace|--config-env)
            [[ ${#args[@]} -ge 2 ]] || exec /usr/bin/git "$@"
            options+=("${args[@]:0:2}")
            args=("${args[@]:2}")
            ;;
        -*)
            options+=("${args[0]}")
            args=("${args[@]:1}")
            ;;
        *) break ;;
    esac
done

# Only the read-only root lookup bypasses ownership; the command trusts one workspace checkout.
if repo=$(/usr/bin/git "${options[@]}" -c safe.directory='*' rev-parse --show-toplevel 2>/dev/null) &&
    repo=$(cd "$repo" 2>/dev/null && pwd -P); then
    case $repo in
        /workspace|/workspace/*)
            exec /usr/bin/git -c safe.directory="$repo" "$@"
            ;;
    esac
fi
exec /usr/bin/git "$@"
