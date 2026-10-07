#!/bin/bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
test_dir=$(mktemp -d)
supervisor=
cleanup() {
    if [[ -n $supervisor ]]; then
        kill -TERM "$supervisor" 2>/dev/null || true
        wait "$supervisor" 2>/dev/null || true
    fi
    rm -rf "$test_dir"
}
trap cleanup EXIT

export TEST_DIR="$test_dir"
export PATH="$test_dir:$PATH"
ln -s "$root/docker/ssh-init.sh" "$test_dir/openwebide-ssh-init"
cat > "$test_dir/child" <<'CHILD'
#!/bin/bash
set -eu
name=${0##*/}
printf '%s\n' "$*" > "$TEST_DIR/$name.args"
echo "$$" > "$TEST_DIR/$name.pid"
trap 'echo TERM > "$TEST_DIR/$name.signal"; exit 0' TERM
trap 'echo INT > "$TEST_DIR/$name.signal"; exit 0' INT
touch "$TEST_DIR/$name.ready"
while [[ ! -f "$TEST_DIR/$name.exit" ]]; do
    sleep 0.05
done
exit "$(cat "$TEST_DIR/$name.exit")"
CHILD
chmod +x "$test_dir/child"
ln -s child "$test_dir/spin"
ln -s child "$test_dir/openwebide-bridge"

await_file() {
    for ((i = 0; i < 200; i++)); do
        [[ -f $1 ]] && return
        sleep 0.05
    done
    echo "Timed out waiting for $1" >&2
    exit 1
}
await_exit() {
    local expected=$1 status=0 name
    # Bound the wait so a broken supervisor cannot hang CI.
    for ((i = 0; i < 200; i++)); do
        if ! kill -0 "$supervisor" 2>/dev/null; then
            wait "$supervisor" || status=$?
            supervisor=
            [[ $status == "$expected" ]]
            for name in spin openwebide-bridge; do
                if [[ -f "$test_dir/$name.pid" ]]; then
                    ! kill -0 "$(cat "$test_dir/$name.pid")" 2>/dev/null
                fi
            done
            return
        fi
        sleep 0.05
    done
    echo 'Supervisor did not exit' >&2
    exit 1
}
start() {
    rm -f "$test_dir/"*.ready "$test_dir/"*.signal "$test_dir/"*.exit \
        "$test_dir/"*.pid "$test_dir/"*.args
    bash "$root/docker/entrypoint.sh" --quiet &
    supervisor=$!
    await_file "$test_dir/spin.ready"
    if [[ ${OPENWEBIDE_BRIDGE:-1} != 0 ]]; then
        await_file "$test_dir/openwebide-bridge.ready"
    fi
}

for name in spin openwebide-bridge; do
    start
    [[ $(cat "$test_dir/spin.args") == 'up --listen 0.0.0.0:3000 --direct-mounts --allow-transient-write --quiet' ]]
    [[ $(cat "$test_dir/openwebide-bridge.args") == '--host 0.0.0.0 --port 3001 --workspace /workspace --backend-url http://127.0.0.1:3000/api --secret-file /app/.spin/bridge-secret' ]]
    echo 37 > "$test_dir/$name.exit"
    await_exit 37
    other=spin
    [[ $name == spin ]] && other=openwebide-bridge
    [[ $(cat "$test_dir/$other.signal") == TERM ]]
done

start
kill -TERM "$supervisor"
await_exit 143
[[ $(cat "$test_dir/spin.signal") == TERM ]]
[[ $(cat "$test_dir/openwebide-bridge.signal") == TERM ]]

export OPENWEBIDE_BRIDGE=0
start
[[ ! -f "$test_dir/openwebide-bridge.ready" ]]
echo 23 > "$test_dir/spin.exit"
await_exit 23

start
kill -TERM "$supervisor"
await_exit 143
[[ $(cat "$test_dir/spin.signal") == TERM ]]

[[ $(bash "$root/docker/entrypoint.sh" printf '%s' dispatched) == dispatched ]]
echo 'Container supervisor tests passed'

# The HTTPS proxy and app share a namespace; only loopback listeners are needed.
export OPENWEBIDE_BRIDGE=1 OPENWEBIDE_APP_HOST=127.0.0.1 OPENWEBIDE_BRIDGE_HOST=127.0.0.1
start
[[ $(cat "$test_dir/spin.args") == 'up --listen 127.0.0.1:3000 --direct-mounts --allow-transient-write --quiet' ]]
[[ $(cat "$test_dir/openwebide-bridge.args") == '--host 127.0.0.1 --port 3001 --workspace /workspace --backend-url http://127.0.0.1:3000/api --secret-file /app/.spin/bridge-secret' ]]
kill -TERM "$supervisor"
await_exit 143
