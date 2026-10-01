#!/bin/bash
set -euo pipefail

workspace_repo=$(mktemp -d /workspace/git-ownership.XXXXXX)
late_repo="$workspace_repo-late"
outside_repo=$(mktemp -d)
test_config=$(mktemp)
test_log=$(mktemp)
supervisor=
cleanup() {
    if [[ -n $supervisor ]]; then
        kill -TERM "$supervisor" 2>/dev/null || true
        wait "$supervisor" 2>/dev/null || true
    fi
    rm -rf "$workspace_repo" "$late_repo" "$outside_repo"
    rm -f "$test_config" "$test_log"
}
trap cleanup EXIT
export GIT_CONFIG_SYSTEM="$test_config"
export GIT_CONFIG_GLOBAL=/dev/null

for repo in "$workspace_repo" "$outside_repo"; do
    git init -q "$repo"
    chown -R 1000:1000 "$repo"
    if /usr/bin/git -C "$repo" status --porcelain=v1 > /dev/null 2>&1; then
        echo 'Expected Git to reject the host-owned checkout before startup' >&2
        exit 1
    fi
done

/app/entrypoint.sh > "$test_log" 2>&1 &
supervisor=$!
ready=0
for ((i = 0; i < 200; i++)); do
    if { exec 3<>/dev/tcp/127.0.0.1/3001; } 2>/dev/null; then
        printf 'GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n' >&3
        if read -r -t 5 response <&3 && [[ $response == *' 200 '* ]]; then
            ready=1
        fi
        exec 3<&- 3>&-
    fi
    [[ $ready == 1 ]] && break
    sleep 0.05
done
if [[ $ready != 1 ]]; then
    cat "$test_log" >&2
    echo 'Bridge did not become ready' >&2
    exit 1
fi

git init -q "$late_repo"
chown -R 1000:1000 "$late_repo"
secret=$(cat /app/.spin/bridge-secret)
for repo in "$workspace_repo" "$late_repo"; do
    git -C "$repo" status --porcelain=v1
    mkdir "$repo/subdir"
    (cd "$repo/subdir" && git status --porcelain=v1)
    [[ $(stat -c '%u:%g' "$repo") == 1000:1000 ]]
    [[ $(stat -c '%u:%g' "$repo/.git") == 1000:1000 ]]
    body="{\"cwd\":\"${repo#/workspace/}\"}"
    exec 3<>/dev/tcp/127.0.0.1/3001
    printf 'POST /git/status HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer %s\r\nContent-Type: application/json\r\nContent-Length: %s\r\nConnection: close\r\n\r\n%s' "$secret" "${#body}" "$body" >&3
    read -r -t 5 response <&3
    exec 3<&- 3>&-
    if [[ $response != *' 200 '* ]]; then
        echo "Bridge Git status failed for $repo: $response" >&2
        cat "$test_log" >&2
        exit 1
    fi
done
if git -C "$outside_repo" status --porcelain=v1 > /dev/null 2>&1; then
    echo 'Git trusted a checkout outside /workspace' >&2
    exit 1
fi
ln -s "$outside_repo" "$workspace_repo/outside"
if git -C "$workspace_repo/outside" status --porcelain=v1 > /dev/null 2>&1; then
    echo 'Git trusted a symlink to a checkout outside /workspace' >&2
    exit 1
fi
echo 'Container Git ownership tests passed'
