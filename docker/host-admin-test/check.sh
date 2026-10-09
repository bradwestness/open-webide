#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
name=openwebide-host-admin-test-$$
fixture_dir=$(mktemp -d)
cleanup() {
    docker rm -f "$name" >/dev/null 2>&1 || true
    docker image rm "$name" >/dev/null 2>&1 || true
    if [ -n "${SSH_AGENT_PID:-}" ]; then ssh-agent -k >/dev/null 2>&1 || true; fi
    rm -rf "$fixture_dir"
}
trap cleanup EXIT
ssh-keygen -q -t ed25519 -N '' -f "$fixture_dir/key"
eval "$(ssh-agent -a "$fixture_dir/agent.sock")" >/dev/null
ssh-add "$fixture_dir/key"
docker build -t "$name" "$root/docker/host-admin-test"
docker run -d --name "$name" --hostname administration-fixture -p 127.0.0.1::22 --mount "type=bind,source=$fixture_dir/key.pub,target=/fixture.pub,readonly" "$name" >/dev/null
for ((i=0; i<100; i++)); do
    if docker exec "$name" test -f /etc/ssh/ssh_host_ed25519_key.pub; then break; fi
    sleep 0.1
done
fixture_port=$(docker port "$name" 22/tcp | sed 's/.*://')
printf '[127.0.0.1]:%s ' "$fixture_port" > "$fixture_dir/known_hosts"
docker exec "$name" cat /etc/ssh/ssh_host_ed25519_key.pub >> "$fixture_dir/known_hosts"
export OPENWEBIDE_TEST_SSH_PORT="$fixture_port"
export OPENWEBIDE_TEST_SSH_KNOWN_HOSTS="$fixture_dir/known_hosts"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"
cd "$root"
cargo test -p openwebide-bridge --test host_admin_ssh -- --ignored --nocapture
