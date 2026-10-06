#!/usr/bin/env bash
set -euo pipefail

repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
if [[ ! -f "$repo/target/wasm32-wasip2/release/openwebide_backend.wasm" || ! -f "$repo/frontend/dist/index.html" ]]; then
    echo 'Build the current backend and frontend with NO_COLOR=true spin build first.' >&2
    exit 1
fi

docker build --file "$repo/tools/editor-view.Dockerfile" \
    --tag open-webide:editor-view-measurements "$repo/tools" >&2
image=$(docker image inspect --format '{{.Id}}' open-webide:editor-view-measurements)
docker run --rm --init --shm-size=1g --memory=10g --cpus=4 \
    --mount "type=bind,src=$repo,dst=/workspace/repos/open-webide" \
    --env "EDITOR_VIEW_IMAGE=$image" \
    "$image" --require-pss "$@"
