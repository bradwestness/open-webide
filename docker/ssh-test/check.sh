#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
image=${1:-openwebide:ci}
name=openwebide-ssh-test-$$
cleanup() {
    docker rm -f "$name" >/dev/null 2>&1 || true
    docker volume rm "$name" >/dev/null 2>&1 || true
    docker network rm "$name" >/dev/null 2>&1 || true
    docker image rm "$name" >/dev/null 2>&1 || true
}
trap cleanup EXIT
docker build -t "$name" "$root/docker/ssh-test"
docker network create "$name" >/dev/null
docker volume create "$name" >/dev/null
docker run -d --name "$name" --network "$name" --network-alias git-ssh-fixture -v "$name:/shared" "$name" >/dev/null
for ((i = 0; i < 100; i++)); do
    if docker exec "$name" test -f /shared/ready; then break; fi
    sleep 0.1
done
docker exec "$name" test -f /shared/ready
# The application sees only public configuration and the agent socket.
docker run --rm --network "$name" \
    --mount "type=volume,source=$name,target=/run/shared,readonly" \
    -e SSH_AUTH_SOCK=/run/shared/agent.sock \
    -v "$root/docker/ssh-test/client.sh:/client.sh:ro" \
    "$image" bash -c 'openwebide-ssh-init /run/shared/public; exec bash /client.sh'
